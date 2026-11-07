# Build and run

Prefer keeping the checkout and Cargo `target/` on `D:\Projects\Project Banter` (or a worktree under `D:\Projects\Project Banter\worktrees\`). Do not use `E:` for Banter builds.

## Prerequisites (Windows)

1. **Rust stable** (project has been built with 1.99.x).
2. **Visual Studio Build Tools** with a C/C++ toolchain. Dear ImGui needs `cl.exe`. Open an **x64 Native Tools** / Developer PowerShell, or run `vcvars64.bat` before `cargo` — `cl.exe` is not on a plain PowerShell PATH.
3. **PostgreSQL** reachable at `BANTER_DATABASE_URL` (default `postgres://banter:banter@127.0.0.1:5432/banter`). For local work: `podman compose up -d postgres` (or Docker Compose). See **Local Postgres port** below when host `5432` is already taken.
4. A display (GLFW opens a real window).

```powershell
cd "D:\Projects\Project Banter"
cargo build -p banter
cargo build -p banter-server
```

Debug binaries land in `target\debug\banter.exe` and `target\debug\banter-server.exe`.

## Local Postgres port

Compose publishes Postgres on host port **`BANTER_PG_PORT`** (default **5432**). On machines where another Postgres already owns `5432` (common on Black-Birb), map Banter’s DB to a free port and point the app at the same host port:

```powershell
cd "D:\Projects\Project Banter"
$env:BANTER_PG_PORT = "5433"
podman compose up -d postgres
# or: docker compose up -d postgres

$env:BANTER_DATABASE_URL = "postgres://banter:banter@127.0.0.1:5433/banter"
cargo run -p banter
# or: cargo run -p banter-server
```

`BANTER_DATABASE_URL` must use the **host** port (`BANTER_PG_PORT`), not the container-internal `5432`. Inside Compose, app containers still talk to `postgres:5432` via the compose default URL. Copy `.env.example` to `.env` if you prefer file-based env (`.env` is gitignored).

Boot applies `schema.sql` then versioned migrations under `crates/banter-server/src/migrations/` so existing `banter-pg` volumes pick up new columns/tables (for example profile customization) without a wipe.

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
# Expect ok, version (Version-*), protocol, min_client_protocol — see docs/version.md
```

## Two clients on one machine

Open the client twice to try voice between accounts. They share one server. With **Stay signed in** checked, `client-session.json` stores one token, so the last account to sign in is restored on the next cold start. Unchecked, Banter keeps the token only in memory for that run and clears any saved file. Both windows can stay signed in during the same run because each keeps its token in memory. Log Out always clears the saved session.

To force the login screen without logging out another session, point `BANTER_DATA` at an empty folder for that process.

## Linux (summary)

Install a C toolchain, GLFW/OpenGL, ALSA or PipeWire, V4L2 headers, and X11 (cursor overlay). On Arch/CachyOS:

```bash
sudo pacman -S --needed base-devel clang cmake pkgconf glfw mesa alsa-lib pipewire libx11
cargo run -p banter
```

Data: `$XDG_DATA_HOME/Banter` or `~/.local/share/Banter`. Speaker loopback and per-app stream audio remain Windows-only; mic, speakers, camera, and screen share work.

## macOS (Apple Silicon)

```
# Local or release: always produce Banter.app (never zip a bare Mach-O).
./scripts/build-macos-release.sh version-1-dev dist/macos
open dist/macos/Banter.app
```

`scripts/package-macos-app.sh` wraps the client binary as **`Banter.app`** (bundle id `foundation.banter.desktop`, `CFBundlePackageType` `APPL`). That is required so Finder launches Banter as a GUI app without opening Terminal, and so System Settings can attribute Microphone / Camera / Screen Recording. Do not ship or double-click a bare `banter-macos-arm` binary. After the first prompt, enable Banter under **System Settings → Privacy & Security**, then quit and reopen the app. Data: `~/Library/Application Support/Banter`. Stream loopback / per-app audio stay Windows/Linux-only. Cut Apple Silicon release zips with Actions workflow `macos-arm-release` (`workflow_dispatch`) or `./scripts/build-macos-release.sh`.

## Tests

```powershell
# Needs Postgres at BANTER_DATABASE_URL (default 127.0.0.1:5432).
# On Black-Birb with Tubecast on 5432, use 5433 as in Local Postgres port.
cargo test -p banter-server
cargo test -p banter
```

Server foundation tests in `crates/banter-server/tests/foundation.rs` are the API contract — do not weaken them. Client coverage includes session, mute/deafen, push-to-talk, media helpers, settings layout rules, and theme metrics.

**CI:** Linux Actions runs the full `banter-server` / `banter` suites against a `postgres:16` service (installs Opus/ALSA/X11/GLFW/Wayland/PipeWire/V4L2 headers). Windows Actions runs **`--lib` unit tests only** (no Postgres service), so sqlx/store integration contracts are Linux-only in CI. The guild-voice SFU RTP loopback unit test is ignored on Windows Actions (no forwarded microphone RTP on that runner) and still runs on Linux.

## Container image (Podman / Docker)

Full walkthrough: **[docker-setup.md](docker-setup.md)**.

Image: Compose builds **`banter-server:local`** from this checkout. **`ghcr.io/snowedinowl/banter`** last landed at `version-1-test-8`. `version-1-test-9` did not push. Older tags stay. Server only — not the desktop app.

```powershell
cd "D:\Projects\Project Banter"
podman compose up -d --build
# or: docker compose up -d --build
Invoke-RestMethod http://127.0.0.1:3970/v1/health
# Expect ok, version (Version-*), protocol, min_client_protocol — see docs/version.md
```

If host port `3970` is taken, publish another (see the setup guide) and point **User Settings → Developer → Remote** at that host port.

Guild voice also needs the slot’s UDP range (`40000-40049` on `banter-a`, `40100-40149` on `banter-b`). `BANTER_ADVERTISE_IP` defaults to `127.0.0.1` for a desktop on this machine. Set it to the host’s reachable address when the client is elsewhere, and open that UDP range. Direct calls do not use it.

Pre-release artifacts: [Version-1-test-13.2](https://github.com/Banter-Foundation/Banter/releases/tag/Version-1-test-13.2). Do not move older release tags.
