# Banter server — Docker / Podman setup

Run **`banter-server`** behind **Caddy** with a separate **PostgreSQL** container and blue/green app slots (`banter-a` / `banter-b`). The desktop Banter app is **not** in the image; build and run that on the host (see [build-and-run.md](build-and-run.md)).

Image name: **`ghcr.io/snowedinowl/banter`** (owner must be lowercase; tags from GitHub Releases, plus `latest`).

**Production / release hosts:** use **Docker Compose v2** (`docker compose`) with [`scripts/deploy-hot-swap.sh`](../scripts/deploy-hot-swap.sh). Podman still works for local Postgres-only or image builds.

## Status (2026-11-03 evening)

Where Arch server + release auto-deploy stand right now:

| Piece | State |
| --- | --- |
| Compose stack (Postgres, Caddy, `banter-a`/`banter-b`) | In repo; ready on the server checkout |
| `scripts/deploy-hot-swap.sh` | In `main` — skips `docker pull` for local-only tags (e.g. `banter-server:local`) |
| `release-deploy.yml` | In `main` — lowercases GHCR owner; pushes with `GHCR_WRITE_TOKEN` (not `GITHUB_TOKEN`) |
| Repo Actions secrets | Set: `DEPLOY_HOST`, `DEPLOY_USER`, `DEPLOY_SSH_KEY`, `DEPLOY_PATH`, `GHCR_READ_TOKEN`, `GHCR_WRITE_TOKEN` |
| GHCR package `banter` | **Not created yet** — every release `build-push` has failed before a successful push |
| Server first boot / hot-swap via Actions | **Blocked** on a successful image push |
| Latest Actions release tried | `Version-1-test-6` — login OK, push failed: *token does not match expected scopes* |
| Manual Podman publish | Last image that landed is `ghcr.io/snowedinowl/banter:version-1-test-8` (+ `:latest` from that publish). `version-1-test-9` through `version-1-test-13.2` did not push. Desktop/server binaries for the current pre-release are `Version-1-test-13.2`. |
| Compose app image | Default is `banter-server:local`, built from this checkout. A published tag is used only when `BANTER_IMAGE` is set and the pull succeeds. |

**Next step (human):** replace `GHCR_WRITE_TOKEN` (and ideally `GHCR_READ_TOKEN`) with a **classic** PAT that has both **`write:packages`** and **`read:packages`** so Actions can push like the manual Podman path. Fine-grained / read-only tokens produce the scopes error. Then re-run a failed workflow. Deploy job has not been reached yet via Actions; SSH secrets are untested until Actions push succeeds.

**Manual bootstrap** (if you need the server up before GHCR works):

```bash
cd "$DEPLOY_PATH"   # e.g. ~/Banter
git pull
./scripts/deploy-hot-swap.sh
```

With `BANTER_IMAGE` unset, the script builds `banter-server:local` from this checkout (`docker build --pull`). If `BANTER_IMAGE` is a registry tag and `docker pull` fails, it builds that same local image and deploys it.

Never put Banter builds or data on `E:`. Prefer `D:\Projects\Project Banter`.

## What you get

| Item | Value |
| --- | --- |
| Binary | `banter-server` only |
| Public entry | Caddy on host **3970** → active `banter-a` or `banter-b` |
| Deploy health | Host **3980** published only while the standby slot is verified |
| Database | Compose service `postgres` → volume `banter-pg` (`BANTER_DATABASE_URL`) |
| Files | Named volume → `/data` (`BANTER_DATA`) — avatars, attachments, board assets |
| Desktop default | Still `127.0.0.1:3970` when Local is selected |

Accounts and messages live in Postgres and stay available if you stop only an app slot. Voice / presence / WebRTC SFU state reset when that app container stops. Uploads stay on `banter-data` and are shared by both slots.

Clean start: this stack does not import old SQLite `banter.db` files.

## Prerequisites

1. **Docker** with Compose v2 on the release host (`docker compose`). Locally, Podman is fine for builds / Postgres-only.
2. Repo checkout on the server (path used as `DEPLOY_PATH`).
3. Free host ports **3970** (public API), **3980** (deploy health), Postgres (**5432**), and the guild-voice UDP ranges **40000-40049** (slot A) and **40100-40149** (slot B), or override via env.

## Quick start (local / first boot)

```bash
cd /path/to/Banter
docker compose up -d --build postgres caddy banter-a
```

That builds `banter-server:local` from this checkout (Compose `pull_policy: build`, and `--pull` on the base images) and starts Postgres, Caddy, and slot A. `--build` matters for Podman, which otherwise keeps an image that is already present.

To hot-swap instead, run `./scripts/deploy-hot-swap.sh` with no `BANTER_IMAGE`. It builds the same local image, starts the first active slot (from [`deploy/active`](../deploy/active), default `a`), checks health on **3980**, points Caddy at it, then drops the temporary **3980** mapping. Clients use **3970** only.

A published image is optional. The last tag on GHCR is still `version-1-test-8`:

```bash
export BANTER_IMAGE=ghcr.io/snowedinowl/banter:version-1-test-8
export BANTER_PULL_POLICY=always
./scripts/deploy-hot-swap.sh
```

If that pull fails, the script builds `banter-server:local` and deploys that.

Health check:

```bash
curl -s http://127.0.0.1:3970/v1/health
# -> {"ok":true}
```

Stop the active app slot (Postgres keeps data):

```bash
docker compose stop banter-a   # or banter-b — see deploy/active
```

Stop everything:

```bash
docker compose down
```

Data stays in `banter-pg` and `banter-data` until you remove volumes (`compose down -v`).

## Release auto-deploy (hot-swap)

On each **GitHub Release** (`published`), [`.github/workflows/release-deploy.yml`](../.github/workflows/release-deploy.yml):

1. Builds and pushes `ghcr.io/<owner>/banter:<lowercase-tag>` and `:latest`.
2. SSHs to the server and runs `./scripts/deploy-hot-swap.sh` with that image.

### Hot-swap flow

1. Pull the new image.
2. Start the **idle** slot (`banter-a` or `banter-b`) with host **3980→3970**.
3. Poll `http://127.0.0.1:3980/v1/health`.
4. Rewrite [`deploy/Caddyfile`](../deploy/Caddyfile) to the standby and reload Caddy (**3970** flips with near-zero downtime).
5. Stop and remove the previous slot; drop the **3980** publish from the new active.

If standby health fails, the current active slot is left serving.

### GitHub secrets

| Secret | Purpose |
| --- | --- |
| `DEPLOY_HOST` | Server hostname or IP |
| `DEPLOY_USER` | SSH user |
| `DEPLOY_SSH_KEY` | Private key for that user |
| `DEPLOY_PATH` | Absolute path to the Banter checkout on the server |
| `GHCR_WRITE_TOKEN` | **Classic** PAT with **`write:packages`** and **`read:packages`** — Actions uses this to push images. `GITHUB_TOKEN` alone cannot create the package on this account |
| `GHCR_READ_TOKEN` | PAT with **`read:packages`** for the server `docker pull` (same classic PAT as write is fine) |

### One-time server setup

1. Install Docker Compose v2; open firewall for **3970** and **3980** (3980 only needs to be reachable from localhost during deploy).
2. Clone this repo to `DEPLOY_PATH` (for example `/opt/banter` or `$HOME/Banter`).
3. Ensure `scripts/deploy-hot-swap.sh` is executable.
4. Set the GitHub secrets above.
5. First deploy: publish a release, or run locally on the server:

```bash
cd "$DEPLOY_PATH"
# Current checkout. Omit BANTER_IMAGE.
./scripts/deploy-hot-swap.sh

# Or a published tag, when GHCR has it:
export BANTER_IMAGE=ghcr.io/snowedinowl/banter:version-1-test-8
export BANTER_PULL_POLICY=always
export GHCR_TOKEN=...   # if the package is private
./scripts/deploy-hot-swap.sh
```

## Local desktop against Compose Postgres

Start only the database, then run the desktop app or a host `banter-server`:

```bash
docker compose up -d postgres
# Default URL:
# postgres://banter:banter@127.0.0.1:5432/banter
```

## Build from source

```bash
docker build --pull -t banter-server:local .
./scripts/deploy-hot-swap.sh
```

With `BANTER_IMAGE` unset, the script builds `banter-server:local` again. Docker cache keeps that short when the checkout has not changed. `BANTER_SKIP_PULL=1` applies only to a registry tag that is already loaded locally.

Or one-shot without Compose (you still need reachable Postgres):

```bash
docker run --rm -p 3970:3970 \
  -e BANTER_BIND=0.0.0.0 \
  -e BANTER_DATA=/data \
  -e BANTER_DATABASE_URL=postgres://banter:banter@host.docker.internal:5432/banter \
  -v banter-data:/data \
  banter-server:local
```

## Environment variables

| Variable | Default | Meaning |
| --- | --- | --- |
| `BANTER_BIND` | `0.0.0.0` | Listen address inside the app container |
| `BANTER_PORT` | `3970` | App listen port (Caddy public host port via compose) |
| `BANTER_DATA` | `/data` | Avatars and uploaded files |
| `BANTER_DATABASE_URL` | `postgres://banter:banter@postgres:5432/banter` (compose) / `postgres://banter:banter@127.0.0.1:5432/banter` (host) | Postgres URL |
| `BANTER_IMAGE` | `banter-server:local` | App image for both slots. Unset builds this checkout. A `registry/name:tag` value is pulled, and a failed pull falls back to a local build. |
| `BANTER_PULL_POLICY` | `build` | Compose pull policy. `build` compiles this checkout. Set `always` together with a registry `BANTER_IMAGE`. The deploy script sets `never` after it has pulled or built. |
| `BANTER_SKIP_PULL` | (unset) | `1` with a registry `BANTER_IMAGE` uses the local image and does not pull or rebuild. |
| `BANTER_PG_PORT` | `5432` | Host port for Postgres |
| `BANTER_UDP_MIN` / `BANTER_UDP_MAX` | `40000`-`40049` on slot A, `40100`-`40149` on slot B | SFU UDP ports. Compose publishes the same range on the host. Required for guild voice, camera, and screen. |
| `BANTER_ADVERTISE_IP` | `127.0.0.1` in Compose | Address written into the SFU’s host ICE candidates. Use the host’s reachable IP when the desktop is not on this machine, and open the UDP range. |
| `GHCR_TOKEN` / `GITHUB_TOKEN` | (none) | Used by deploy script to `docker login ghcr.io` |

## Files in the repo

| File | Role |
| --- | --- |
| `Dockerfile` | Multi-stage build: Rust → slim runtime with `banter-server` |
| `docker-compose.yml` | `postgres`, `caddy`, `banter-a`, `banter-b` |
| `deploy/Caddyfile` | Reverse proxy to the active slot (rewritten by deploy) |
| `deploy/active` | `a` or `b` — which slot Caddy should use |
| `scripts/deploy-hot-swap.sh` | Pull, health on 3980, flip Caddy, retire old slot |
| `.github/workflows/release-deploy.yml` | Release → GHCR → SSH deploy |
| `.dockerignore` | Keeps target/, secrets, docs out of the build context |

## Troubleshooting

| Symptom | What to try |
| --- | --- |
| Port already allocated | Free 3970/3980 or set `BANTER_PORT` for Caddy’s host port |
| Health fails on 3980 | `docker compose logs banter-a` / `banter-b`; confirm Postgres is healthy |
| Caddy 502 | Check `deploy/active` and that that slot is running; re-run deploy script |
| Cannot pull from GHCR | Set `GHCR_READ_TOKEN` / `GHCR_TOKEN` with `read:packages`; package must exist and be public or grant the token Read |
| Push: `repository name must be lowercase` | Fixed on `main` — image owner is lowercased in the workflow |
| Push: `permission_denied: read_package` (with `GITHUB_TOKEN`) | Expected on this user account — use `GHCR_WRITE_TOKEN` |
| Push: `token does not match expected scopes` | Classic PAT missing **`write:packages`** — recreate token, update secret, re-run |
| Local `BANTER_IMAGE=banter-server:local` tries Docker Hub | Update to this script: a tag with no `/` is built here and is not pulled |
| Client won’t connect | Developer → Remote must use host **3970** (Caddy), not 3980 |
| Want a clean DB | `docker compose down -v` (deletes Postgres and file volumes) |

## Related docs

- [build-and-run.md](build-and-run.md) — host builds, tests, containers
- [architecture.md](architecture.md) — crates and server layout
- [current-build.md](current-build.md) — what’s on this machine right now
