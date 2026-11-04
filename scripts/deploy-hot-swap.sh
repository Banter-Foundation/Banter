#!/usr/bin/env bash
# Hot-swap banter-a / banter-b behind Caddy on :3970.
# Standby is published on :3980 for health checks, then Caddy flips, then the old slot stops.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

COMPOSE=(docker compose)
ACTIVE_FILE="$ROOT/deploy/active"
CADDYFILE="$ROOT/deploy/Caddyfile"
STANDBY_OVERRIDE="$ROOT/deploy/compose.standby.yml"
HEALTH_URL="${BANTER_HEALTH_URL:-http://127.0.0.1:3980/v1/health}"
HEALTH_TIMEOUT_SEC="${BANTER_HEALTH_TIMEOUT_SEC:-60}"

if [[ -z "${BANTER_IMAGE:-}" ]]; then
  echo "BANTER_IMAGE is required (e.g. ghcr.io/snowedinowl/banter:version-1-test-3)" >&2
  exit 1
fi

log() { printf '[deploy] %s\n' "$*"; }

login_ghcr() {
  local token="${GHCR_TOKEN:-${GITHUB_TOKEN:-}}"
  if [[ -n "$token" ]]; then
    log "logging in to ghcr.io"
    printf '%s' "$token" | docker login ghcr.io -u "${GHCR_USER:-$(whoami)}" --password-stdin
  else
    log "no GHCR_TOKEN/GITHUB_TOKEN; assuming image is pullable without login"
  fi
}

read_active() {
  if [[ -f "$ACTIVE_FILE" ]]; then
    tr -d '[:space:]' <"$ACTIVE_FILE"
  else
    printf 'a'
  fi
}

write_active() {
  printf '%s\n' "$1" >"$ACTIVE_FILE"
}

other_slot() {
  if [[ "$1" == "a" ]]; then
    printf 'b'
  else
    printf 'a'
  fi
}

service_running() {
  local id
  id="$("${COMPOSE[@]}" ps -q "banter-$1" 2>/dev/null || true)"
  [[ -n "$id" ]]
}

write_caddy_upstream() {
  local slot="$1"
  cat >"$CADDYFILE" <<EOF
# Public entry for Banter. Managed by scripts/deploy-hot-swap.sh — do not edit by hand on the server.
:3970 {
	reverse_proxy banter-${slot}:3970
}
EOF
}

write_standby_override() {
  local slot="$1"
  cat >"$STANDBY_OVERRIDE" <<EOF
services:
  banter-${slot}:
    ports:
      - "3980:3970"
EOF
}

wait_healthy() {
  local deadline=$((SECONDS + HEALTH_TIMEOUT_SEC))
  log "waiting for health at $HEALTH_URL (timeout ${HEALTH_TIMEOUT_SEC}s)"
  while (( SECONDS < deadline )); do
    if curl -fsS "$HEALTH_URL" | grep -q '"ok"[[:space:]]*:[[:space:]]*true'; then
      log "standby is healthy"
      return 0
    fi
    sleep 2
  done
  echo "standby failed health check within ${HEALTH_TIMEOUT_SEC}s" >&2
  return 1
}

reload_caddy() {
  log "reloading Caddy"
  "${COMPOSE[@]}" exec -T caddy caddy reload --config /etc/caddy/Caddyfile
}

# Caddyfile is mounted :ro — remount by recreating caddy after rewrite, then reload is enough if writable.
# Compose marks the file ro; rewrite on host is visible; reload picks it up.
ensure_infra() {
  log "ensuring postgres + caddy"
  "${COMPOSE[@]}" up -d postgres caddy
}

start_slot() {
  local slot="$1"
  local with_standby_port="${2:-0}"
  export BANTER_IMAGE
  if [[ "$with_standby_port" == "1" ]]; then
    write_standby_override "$slot"
    log "starting banter-$slot with image $BANTER_IMAGE (published on :3980)"
    "${COMPOSE[@]}" -f docker-compose.yml -f "$STANDBY_OVERRIDE" up -d --no-deps "banter-$slot"
  else
    rm -f "$STANDBY_OVERRIDE"
    log "starting banter-$slot with image $BANTER_IMAGE"
    "${COMPOSE[@]}" up -d --no-deps "banter-$slot"
  fi
}

stop_slot() {
  local slot="$1"
  log "stopping banter-$slot"
  "${COMPOSE[@]}" stop "banter-$slot" || true
  "${COMPOSE[@]}" rm -f "banter-$slot" || true
}

drop_standby_port() {
  local slot="$1"
  log "dropping :3980 publish from banter-$slot"
  rm -f "$STANDBY_OVERRIDE"
  export BANTER_IMAGE
  "${COMPOSE[@]}" up -d --no-deps --force-recreate "banter-$slot"
}

main() {
  login_ghcr
  log "pulling $BANTER_IMAGE"
  docker pull "$BANTER_IMAGE"

  local active standby
  active="$(read_active)"
  if [[ "$active" != "a" && "$active" != "b" ]]; then
    active=a
  fi
  standby="$(other_slot "$active")"

  # Keep Caddy in sync with deploy/active (git pull may reset Caddyfile).
  write_caddy_upstream "$active"
  ensure_infra

  local active_up=0 standby_up=0
  if service_running "$active"; then active_up=1; fi
  if service_running "$standby"; then standby_up=1; fi

  # Bootstrap: nothing running yet → bring up active slot only.
  if [[ "$active_up" -eq 0 && "$standby_up" -eq 0 ]]; then
    log "bootstrap: starting banter-$active as first active slot"
    write_caddy_upstream "$active"
    start_slot "$active" 1
    if ! wait_healthy; then
      stop_slot "$active"
      exit 1
    fi
    write_caddy_upstream "$active"
    "${COMPOSE[@]}" up -d --force-recreate caddy
    drop_standby_port "$active"
    write_active "$active"
    log "bootstrap complete; active=banter-$active"
    exit 0
  fi

  # If marked active is down but the other is up, treat the running one as active.
  if [[ "$active_up" -eq 0 && "$standby_up" -eq 1 ]]; then
    log "active banter-$active is down; treating banter-$standby as active"
    active="$standby"
    standby="$(other_slot "$active")"
    write_active "$active"
    write_caddy_upstream "$active"
    "${COMPOSE[@]}" up -d --force-recreate caddy
  fi

  log "hot-swap: active=banter-$active → standby=banter-$standby"
  stop_slot "$standby"
  start_slot "$standby" 1

  if ! wait_healthy; then
    log "leaving active banter-$active in place; removing failed standby"
    stop_slot "$standby"
    rm -f "$STANDBY_OVERRIDE"
    exit 1
  fi

  write_caddy_upstream "$standby"
  reload_caddy || {
    log "reload failed; recreating caddy"
    "${COMPOSE[@]}" up -d --force-recreate caddy
  }

  # Confirm public entry still healthy through Caddy.
  local public="http://127.0.0.1:${BANTER_PORT:-3970}/v1/health"
  log "checking public health at $public"
  local deadline=$((SECONDS + 30))
  while (( SECONDS < deadline )); do
    if curl -fsS "$public" | grep -q '"ok"[[:space:]]*:[[:space:]]*true'; then
      break
    fi
    sleep 1
  done
  if ! curl -fsS "$public" | grep -q '"ok"[[:space:]]*:[[:space:]]*true'; then
    echo "public health check failed after flip; attempting rollback to banter-$active" >&2
    write_caddy_upstream "$active"
    reload_caddy || "${COMPOSE[@]}" up -d --force-recreate caddy
    stop_slot "$standby"
    rm -f "$STANDBY_OVERRIDE"
    exit 1
  fi

  write_active "$standby"
  stop_slot "$active"
  drop_standby_port "$standby"
  rm -f "$STANDBY_OVERRIDE"

  log "pruning dangling images (keeping $BANTER_IMAGE)"
  docker image prune -f >/dev/null || true

  log "done; active=banter-$standby image=$BANTER_IMAGE"
}

main "$@"
