# Banter server — Docker / Podman setup

Run **`banter-server`** in a container. The desktop Banter app is **not** in the image; build and run that on the host (see [build-and-run.md](build-and-run.md)).

Image name: **`ghcr.io/snowedinowl/banter`**  
Published pre-release tag: **`version-1-test-1`** (GitHub release `Version-1-test-1`).

Podman and Docker both work. On Black-Birb, prefer **Podman** when Docker Desktop’s engine is not running.

Never put Banter builds or data on `E:`. Prefer `D:\Projects\Project Banter`.

## What you get

| Item | Value |
| --- | --- |
| Binary | `banter-server` only |
| Bind inside container | `0.0.0.0` (`BANTER_BIND`) |
| Port | `3970` (`BANTER_PORT`, overridable) |
| Data | Named volume → `/data` (`BANTER_DATA`) — SQLite `banter.db`, uploads |
| Desktop default | Still `127.0.0.1:3970` when Local is selected |

Voice / presence / WebRTC SFU state reset when the container stops. Accounts and messages persist on the volume.

## Prerequisites

1. **Podman** or **Docker** with Compose (`podman compose` or `docker compose`).
2. Repo checkout at `D:\Projects\Project Banter` (or your clone).
3. Free host port, or pick another (see below). A local Banter window often already holds `3970`.

## Quick start (published image)

```powershell
cd "D:\Projects\Project Banter"

# Podman
podman compose up -d

# Docker
docker compose up -d
```

Compose pulls/builds image `ghcr.io/snowedinowl/banter:version-1-test-1` by default (`BANTER_IMAGE` overrides).

Health check:

```powershell
Invoke-RestMethod http://127.0.0.1:3970/v1/health
# -> { "ok": true }
```

Stop:

```powershell
podman compose down
# or: docker compose down
```

Data stays in the `banter-data` volume until you remove it (`compose down -v`).

## Build from source

```powershell
cd "D:\Projects\Project Banter"

podman build -t banter-server:local .
# or: docker build -t banter-server:local .

# Run that local tag through compose
$env:BANTER_IMAGE = "localhost/banter-server:local"   # Podman local tag
podman compose up -d --build
```

Or one-shot without compose:

```powershell
podman run --rm -p 3970:3970 `
  -e BANTER_BIND=0.0.0.0 `
  -e BANTER_DATA=/data `
  -v banter-data:/data `
  banter-server:local
```

## Different host port

If `3970` is taken (desktop Banter or another server):

```powershell
# Publish host 3971 → container 3970
podman run --rm -p 3971:3970 -e BANTER_BIND=0.0.0.0 ghcr.io/snowedinowl/banter:version-1-test-1

# Or with compose: set BANTER_PORT so both sides match (compose maps host:container to that port)
$env:BANTER_PORT = "3971"
podman compose up -d
```

Then check `http://127.0.0.1:3971/v1/health`.

## Point the desktop client at the container

1. Open Banter on the host.
2. **User Settings → Developer → Remote**.
3. Enter `127.0.0.1:3971` (or whatever host port you published), or `http://127.0.0.1:3971`.
4. **Apply**. Sign in / register against that server.

**Local** mode still uses the embedded server on `127.0.0.1:3970` and does not start when you intentionally use Remote.

## Environment variables

| Variable | Default in image / compose | Meaning |
| --- | --- | --- |
| `BANTER_BIND` | `0.0.0.0` | Listen address inside the container |
| `BANTER_PORT` | `3970` | Listen port (and compose publish mapping) |
| `BANTER_DATA` | `/data` | Directory for SQLite and files |
| `BANTER_IMAGE` | `ghcr.io/snowedinowl/banter:version-1-test-1` | Compose image name (local builds override) |

A non-empty CLI address on the binary still wins over env (same rules as a host `banter-server`).

## Files in the repo

| File | Role |
| --- | --- |
| `Dockerfile` | Multi-stage build: Rust → slim runtime with `banter-server` |
| `docker-compose.yml` | Service, port publish, named volume |
| `.dockerignore` | Keeps target/, secrets, docs, agent scratch out of the build context |

The Dockerfile stubs the desktop `banter` crate so the workspace resolves without compiling GLFW/ImGui.

## Troubleshooting

| Symptom | What to try |
| --- | --- |
| Port already allocated | Use another host port (`-p 3971:3970` or `BANTER_PORT=3971`) |
| `docker` not found / Desktop engine down | Use `podman` / `podman compose` |
| Health fails | `podman compose logs` / `docker compose logs`; confirm publish mapping |
| Client won’t connect | Developer → Remote must match the **host** port; rebuild desktop if Remote UI is missing |
| Want a clean DB | `podman compose down -v` (deletes the volume) |

## Related docs

- [build-and-run.md](build-and-run.md) — host builds, tests, Linux notes  
- [architecture.md](architecture.md) — crates and server layout  
- [current-build.md](current-build.md) — what’s on this machine right now  
