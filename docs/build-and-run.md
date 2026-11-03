# Build and run

Prefer keeping the checkout and Cargo `target/` on `D:\Projects\Project Banter` (or a worktree under `D:\Projects\Project Banter\worktrees\`). Do not use `E:` for Banter builds.

## Prerequisites (Windows)

1. **Rust stable** (project has been built with 1.99.x).
2. **Visual Studio Build Tools** with a C/C++ toolchain. SQLite (bundled) and Dear ImGui need `cl.exe`. Open an **x64 Native Tools** / Developer PowerShell, or run `vcvars64.bat` before `cargo` — `cl.exe` is not on a plain PowerShell PATH.
3. A display (GLFW opens a real window).

```powershell
cd "D:\Projects\Project Banter"
cargo build -p banter
cargo build -p banter-server
```

Debug binaries land in `target\debug\banter.exe` and `target\debug\banter-server.exe`.

## Run the desktop client (Windows)

```powershell
cd "D:\Projects\Project Banter"
cargo run -p banter
```

Or launch `target\debug\banter.exe` after a build. Data goes to `%LOCALAPPDATA%\Banter` unless `BANTER_DATA` is set.

If Developer is set to Local (the default) and port `3970` is free, the client starts an embedded server. If another Banter or `banter-server` already holds `127.0.0.1:3970`, the client uses that instance.

### Developer: Local vs Remote

Open **User Settings → Developer**:

- **Local** — `http://127.0.0.1:3970` (`banter_server::DEFAULT_ADDR`). Starts the embedded server when nothing is listening.
- **Remote** — enter a host, `host:port`, or full `http://` / `https://` URL, then **Apply** (or Enter). Invalid input stays on the previous base and shows an error.
- Choice persists in `%LOCALAPPDATA%\Banter\dev-server.json` (or the `BANTER_DATA` folder). Switching reconnects the HTTP client and WebSocket; if the session token is invalid on the new host, Banter clears the local session and asks you to sign in again.

## Run the server alone (Windows)

```powershell
cargo run -p banter-server
```

Examples:

```powershell
# Explicit bind (wins over env)
cargo run -p banter-server -- 0.0.0.0:3970

# Env-only (no CLI address)
$env:BANTER_BIND = "0.0.0.0"
$env:BANTER_PORT = "3970"
cargo run -p banter-server
```

Health check:

```powershell
Invoke-RestMethod http://127.0.0.1:3970/v1/health
# -> { "ok": true }
```

## Two clients on one machine

Open the client twice to try voice between accounts. They share one server. `client-session.json` stores one token, so the last account to sign in is restored on the next cold start. Both windows can stay signed in during the same run because each keeps its token in memory.

To force the login screen without logging out another session, point `BANTER_DATA` at an empty folder for that process.

## Linux (summary)

Install a C toolchain, GLFW/OpenGL, ALSA or PipeWire, V4L2 headers, and X11 (cursor overlay). On Arch/CachyOS:

```bash
sudo pacman -S --needed base-devel clang cmake pkgconf glfw mesa alsa-lib pipewire libx11
cargo run -p banter
```

Data: `$XDG_DATA_HOME/Banter` or `~/.local/share/Banter`. Speaker loopback and per-app stream audio remain Windows-only; mic, speakers, camera, and screen share work.

## Tests

```powershell
cargo test -p banter-server
cargo test -p banter
```

Server foundation tests in `crates/banter-server/tests/foundation.rs` are the API contract — do not weaken them. Client coverage includes session, mute/deafen, push-to-talk, media helpers, settings layout rules, and theme metrics.

## Container image (Podman / Docker)

Image: **`ghcr.io/snowedinowl/banter`**  
Published tag used for the first pre-release: **`version-1-test-1`** (also GitHub release / tag `Version-1-test-1`).

The image contains **`banter-server` only**, not the desktop app. The build stage stubs the `banter` crate so the workspace manifest resolves without compiling GLFW/ImGui.

### Compose

Repo root has `Dockerfile`, `docker-compose.yml`, and `.dockerignore`.

```powershell
# Podman (preferred on Black-Birb when Docker Desktop's engine is down)
podman compose up

# Docker
docker compose up
```

Compose defaults:

- Image `ghcr.io/snowedinowl/banter:version-1-test-1` (or local build from the Dockerfile)
- `BANTER_BIND=0.0.0.0`, port `3970` (override host/container port with `BANTER_PORT`)
- Named volume `banter-data` → `/data`
- Optional secret via `BANTER_SECRET_FILE` (never baked into the image)

Pull and run without compose:

```powershell
podman run --rm -p 3970:3970 -e BANTER_BIND=0.0.0.0 ghcr.io/snowedinowl/banter:version-1-test-1
```

If host port `3970` is already taken by a desktop Banter window, publish another port (for example `3971:3970`) and hit that URL for API checks. Point the desktop client at it from **User Settings → Developer → Remote** (for example `127.0.0.1:3971`).

### Pre-release artifacts

GitHub release [Version-1-test-1](https://github.com/SnowedInOwl/Banter/releases/tag/Version-1-test-1) includes Windows zip / `banter-server.exe` attachments. Do not move that tag for later UI-only commits.
