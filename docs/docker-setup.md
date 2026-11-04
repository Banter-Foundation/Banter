# Banter server — Docker / Podman setup

Run **`banter-server`** behind **Caddy** with a separate **PostgreSQL** container and blue/green app slots (`banter-a` / `banter-b`). The desktop Banter app is **not** in the image; build and run that on the host (see [build-and-run.md](build-and-run.md)).

Image name: **`ghcr.io/snowedinowl/banter`** (tags from GitHub Releases, plus `latest`).
Published pre-release tag: **`version-1-test-4`** (earlier tags remain on GHCR / Releases).

**Production / release hosts:** use **Docker Compose v2** (`docker compose`) with [`scripts/deploy-hot-swap.sh`](../scripts/deploy-hot-swap.sh). Podman still works for local Postgres-only or image builds.

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
3. Free host ports **3970** (public API), **3980** (deploy health), and Postgres (**5432**), or override via env.

## Quick start (local / first boot)

```bash
cd /path/to/Banter
docker compose up -d postgres caddy
export BANTER_IMAGE=ghcr.io/snowedinowl/banter:latest
./scripts/deploy-hot-swap.sh
```

The deploy script pulls the image, starts the first active slot (from [`deploy/active`](../deploy/active), default `a`), checks health on **3980**, points Caddy at it, then drops the temporary **3980** mapping. Clients use **3970** only.

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
| `GHCR_READ_TOKEN` | PAT (or fine-grained token) with `read:packages` so the server can `docker pull` private GHCR images |

Workflow permissions already include `packages: write` for the push job (`GITHUB_TOKEN`).

### One-time server setup

1. Install Docker Compose v2; open firewall for **3970** and **3980** (3980 only needs to be reachable from localhost during deploy).
2. Clone this repo to `DEPLOY_PATH` (for example `/opt/banter` or `$HOME/Banter`).
3. Ensure `scripts/deploy-hot-swap.sh` is executable.
4. Set the GitHub secrets above.
5. First deploy: publish a release, or run locally on the server:

```bash
cd "$DEPLOY_PATH"
export BANTER_IMAGE=ghcr.io/snowedinowl/banter:latest
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
docker build -t banter-server:local .
export BANTER_IMAGE=banter-server:local
./scripts/deploy-hot-swap.sh
```

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
| `BANTER_IMAGE` | `ghcr.io/snowedinowl/banter:latest` | App image for both slots |
| `BANTER_PG_PORT` | `5432` | Host port for Postgres |
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
| Cannot pull from GHCR | Set `GHCR_READ_TOKEN` / `GHCR_TOKEN` with `read:packages` |
| Client won’t connect | Developer → Remote must use host **3970** (Caddy), not 3980 |
| Want a clean DB | `docker compose down -v` (deletes Postgres and file volumes) |

## Related docs

- [build-and-run.md](build-and-run.md) — host builds, tests, containers
- [architecture.md](architecture.md) — crates and server layout
- [current-build.md](current-build.md) — what’s on this machine right now
