# Build and run

Prefer keeping the checkout and Cargo `target/` on `D:\Projects\Project Banter` (or a worktree under `D:\Projects\Project Banter\worktrees\`). Do not use `E:` for Banter builds.

## Prerequisites (Windows)

1. **Rust stable** (project has been built with 1.99.x).
2. **Visual Studio Build Tools** with a C/C++ toolchain. Dear ImGui needs `cl.exe`. Open an **x64 Native Tools** / Developer PowerShell, or run `vcvars64.bat` before `cargo` — `cl.exe` is not on a plain PowerShell PATH.
3. **PostgreSQL** reachable at `BANTER_DATABASE_URL` (default `postgres://banter:banter@127.0.0.1:5432/banter`). For local work: `podman compose up -d postgres` (or Docker Compose).
4. A display (GLFW opens a real window).

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

Startup never blocks the window on a missing local server. Behavior:

- **Already logged in on Local** — Banter tries to start the embedded server (or reuse whatever already holds `127.0.0.1:3970`). If that fails, the UI opens and asks for a remote instead of exiting.
- **Fresh / not logged in on Local** — if something is already listening on `3970`, the client uses it; otherwise it opens and asks for a remote (it does not force-start local).
- **Remote already selected** — connect to that host; local is not started.
- **User picks Local** in Developer / Server options — then Banter tries to start the embedded server.

### Developer: Local vs Remote

Open **User Settings → Developer** (or the gear on the sign-in screen):

- **Local** — `http://127.0.0.1:3970` (`banter_server::DEFAULT_ADDR`). Starts the embedded server when you select Local (or when restoring a logged-in Local session).
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

Full walkthrough: **[docker-setup.md](docker-setup.md)**.

Image: **`ghcr.io/snowedinowl/banter`** (`version-1-test-4.1` for the current pre-release; older `version-1-test-1` / `version-1-test-2` / `version-1-test-3` / `version-1-test-4` tags stay). Server only — not the desktop app.

```powershell
cd "D:\Projects\Project Banter"
podman compose up -d
# or: docker compose up -d
Invoke-RestMethod http://127.0.0.1:3970/v1/health
```

If host port `3970` is taken, publish another (see the setup guide) and point **User Settings → Developer → Remote** at that host port.

Pre-release artifacts: [Version-1-test-4.1](https://github.com/SnowedInOwl/Banter/releases/tag/Version-1-test-4.1). Do not move older release tags for later UI-only commits.
