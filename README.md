# Banter

Banter is a local-first desktop chat app with a Discord-like layout (temporary skin). A Rust client (GLFW, OpenGL 3.3, Dear ImGui docking) talks to a Rust `banter-server` over HTTP and WebSocket. Accounts, servers, channels, and messages live in **PostgreSQL**; uploaded files stay under `BANTER_DATA`. Voice membership, calls, and presence are in memory and reset when the server process stops.

This README is a short front door for **users** and **developers**. Longer guides live under [`docs/`](docs/).

---

## For users

### What you get

- Register / sign in with a local username and password
- Servers, invites, text and voice channels, friends, and 1:1 DMs / calls
- Mute, deafen, push-to-talk, camera, and screen share (Go Live)
- Chat text uses **BFMD** (not CommonMark) — see [`docs/bfmd.md`](docs/bfmd.md)

Honest “what works / what’s deferred”: [`docs/feature-status.md`](docs/feature-status.md) and [`docs/foundation.md`](docs/foundation.md).

The Discord-like chrome is a temporary skin — not final Banter branding, and no Discord logos.

### Platforms

| Platform | Client | Notes |
| --- | --- | --- |
| **Windows** | Desktop `banter` | Primary day-to-day target on Owl’s machine |
| **Linux** | Desktop `banter` | X11 / PipeWire-ready; speaker loopback / per-app stream audio stay Windows-only |
| **macOS (Apple Silicon)** | `Banter.app` | Mic / Camera / Screen Recording grants required; loopback / per-app audio not available |

Android and website work live on permanent platform branches (`android`, `website`) — see [`docs/permanent-branches.md`](docs/permanent-branches.md). This repo’s mainline is the **desktop** client + server. There is no App Store / paid Apple distribution path documented for Banter here.

### Get a pre-release build

Pre-releases are tagged `Version-1-test-N` on GitHub Releases.

- **Current label:** [Version-1-test-13.4](https://github.com/Banter-Foundation/Banter/releases/tag/Version-1-test-13.4)  
  Attached assets today: Linux client zip + `banter-server-linux`, and macOS Apple Silicon `Banter.app` zip + `banter-server-macos-arm`.
- **Windows zip** for the previous full cut: [Version-1-test-13.3](https://github.com/Banter-Foundation/Banter/releases/tag/Version-1-test-13.3) (`Banter-windows-version-1-test-13-3.zip` + `banter-server.exe`), or build from source below.

Always check the release page for the exact asset list — cuts sometimes ship Linux-only or defer a platform.

**macOS:** open **`Banter.app`** from the zip (double-click or `open Banter.app`). Do not run a bare Mach-O — Finder would open Terminal. On first mic / camera / screen share, enable Banter under **System Settings → Privacy & Security**, then quit and reopen.

### Basic expectations

- Default API address is `http://127.0.0.1:3970` (**User Settings → Developer → Local**). You can switch to a **Remote** host (`host`, `host:port`, or URL); the choice is saved in `dev-server.json` under the data directory.
- Local mode needs **PostgreSQL** reachable at `BANTER_DATABASE_URL` (default `postgres://banter:banter@127.0.0.1:5432/banter`). Compose Postgres is the usual local path — see [Build from source](#build-from-source) and [`docs/docker-setup.md`](docs/docker-setup.md).
- Data directory (unless `BANTER_DATA` is set):

  | OS | Path |
  | --- | --- |
  | Windows | `%LOCALAPPDATA%\Banter` |
  | Linux | `$XDG_DATA_HOME/Banter` or `~/.local/share/Banter` |
  | macOS | `~/Library/Application Support/Banter` |

- Voice / presence / open WebSockets reset when the server process stops; chat history in Postgres does not.
- File attach in the composer may show **Currently Unavailable**; existing attachments still download. See feature status.

---

## For developers

### Clone

```bash
git clone https://github.com/Banter-Foundation/Banter.git
cd Banter
```

On Owl’s Windows machine the known checkout path is:

```text
D:\Projects\Project Banter
```

Keep the repo and Cargo `target/` there (or under `D:\Projects\Project Banter\worktrees\`). **Never** use `E:` for Banter builds or data.

### Prerequisites

1. **Rust stable** (built with 1.99.x).
2. **C/C++ toolchain** — Dear ImGui needs a native compiler (`cl.exe` on Windows via VS Build Tools / x64 Native Tools; `base-devel` / clang on Linux).
3. **PostgreSQL 16** reachable at `BANTER_DATABASE_URL`. Easiest local start:

   ```bash
   # From the repo root — Podman or Docker Compose v2
   podman compose up -d postgres
   # or: docker compose up -d postgres
   ```

   Default URL: `postgres://banter:banter@127.0.0.1:5432/banter`. If host `5432` is taken, set `BANTER_PG_PORT` and point `BANTER_DATABASE_URL` at that host port (see [`docs/build-and-run.md`](docs/build-and-run.md)). Copy [`.env.example`](.env.example) to `.env` if you prefer file-based env (`.env` is gitignored).
4. A **display** for the desktop client (GLFW opens a real window).

Linux package sketch (Arch / CachyOS):

```bash
sudo pacman -S --needed base-devel clang cmake pkgconf glfw mesa alsa-lib pipewire libx11
```

### Build from source

```bash
# Windows known path:
#   cd "D:\Projects\Project Banter"

cargo build -p banter
cargo build -p banter-server
```

Debug binaries: `target/debug/banter` (`banter.exe` on Windows) and `target/debug/banter-server`.

### Run client + server

**Desktop client** (preferred day-to-day):

```bash
cargo run -p banter
```

Startup does not hard-fail if nothing is on `3970`. Local vs Remote is **User Settings → Developer** (also the gear on the sign-in screen). Embedded local server always binds `127.0.0.1:3970` when Local is selected / restored.

**Server alone:**

```bash
cargo run -p banter-server
# Optional bind wins over env:
# cargo run -p banter-server -- 0.0.0.0:3970
```

Listen address: first CLI argument, else `BANTER_BIND` / `BANTER_PORT` (defaults `0.0.0.0` / `3970` when either env is set), else `127.0.0.1:3970`. Details: [`docs/overview.md`](docs/overview.md).

Health:

```bash
curl -s http://127.0.0.1:3970/v1/health
# Expect ok, version (Version-*), protocol, min_client_protocol — docs/version.md
```

**Two clients on one machine:** run the client twice to try voice between accounts. They share one server. With **Stay signed in** checked, `client-session.json` stores one token (last writer wins on next cold start). Point `BANTER_DATA` at an empty folder to force a fresh login without logging out another window.

Full walkthrough: [`docs/build-and-run.md`](docs/build-and-run.md).

### Server container

Compose stack: Postgres + Caddy + blue/green `banter-a` / `banter-b`. The image is **`banter-server` only** (not the desktop app).

```bash
podman compose up -d --build
# or: docker compose up -d --build
```

Default image is `banter-server:local` from this checkout. Last GHCR tag that landed is `ghcr.io/snowedinowl/banter:version-1-test-8` (newer tags have not pushed — see status table in [`docs/docker-setup.md`](docs/docker-setup.md)). Point the desktop client at the published host port with **Developer → Remote**.

Guild voice needs the slot’s UDP range and `BANTER_ADVERTISE_IP` when the client is not on the same host — see the docker guide.

### Workspace layout

```text
Banter/
  crates/banter/            # desktop client
  crates/banter-server/     # API + PostgreSQL + live state
  testing-debug/            # headless Linux stream debug CLI (workspace member)
  docs/                     # collaborator guides
  deploy/                   # Caddyfile + active slot for hot-swap
  scripts/                  # macOS packaging, deploy-hot-swap, checks
  Dockerfile                # server image
  docker-compose.yml
  .github/workflows/        # CI + release/deploy helpers
```

Module map: [`docs/architecture.md`](docs/architecture.md).

### Tests and CI

```bash
cargo test -p banter-server   # needs Postgres at BANTER_DATABASE_URL
cargo test -p banter
```

Do not weaken `crates/banter-server/tests/foundation.rs` — that suite is the API contract.

[`.github/workflows/ci.yml`](.github/workflows/ci.yml) runs:

- **Linux:** full `banter-server` / `banter` suites against a `postgres:16` service, plus a macOS `.app` zip layout check
- **Windows:** `--lib` unit tests only (no Postgres service on that job)

Release deploy / GHCR push: [`.github/workflows/release-deploy.yml`](.github/workflows/release-deploy.yml) (blocked until a classic PAT with `write:packages` lands — details in docker-setup).

### Docs index

| Doc | Contents |
| --- | --- |
| [docs/overview.md](docs/overview.md) | Product, stack, data paths, bind address |
| [docs/build-and-run.md](docs/build-and-run.md) | Windows / Linux / macOS build & run, tests |
| [docs/docker-setup.md](docs/docker-setup.md) | Podman / Docker server, hot-swap, GHCR status |
| [docs/architecture.md](docs/architecture.md) | Crates and modules |
| [docs/feature-status.md](docs/feature-status.md) | What works vs deferred (honest) |
| [docs/foundation.md](docs/foundation.md) | Chosen foundation slice + deferred list |
| [docs/version.md](docs/version.md) | Release label + protocol floors |
| [docs/errors.md](docs/errors.md) | Stable `BANTER_*` codes |
| [docs/bfmd.md](docs/bfmd.md) | Chat text formatting |
| [docs/emote-library.md](docs/emote-library.md) | Emote / sticker / GIF packs |
| [docs/chat-commands.md](docs/chat-commands.md) | Composer slash / prefix commands |
| [docs/chat-autosuggest.md](docs/chat-autosuggest.md) | Compose autosuggest |
| [docs/streaming.md](docs/streaming.md) | Voice / stream media notes |
| [docs/contributing.md](docs/contributing.md) | Branches, worktrees, commits, changelog |
| [docs/permanent-branches.md](docs/permanent-branches.md) | Long-lived `linux` / `windows` / `mac` / `android` / `website` |
| [docs/modding/modding.md](docs/modding/modding.md) | Mods / plugins / bots (spec; not loaded yet) |
| [docs/current-build.md](docs/current-build.md) | Live observation notes |

### Contributing

Read [`docs/contributing.md`](docs/contributing.md) before changing code. Short rules:

- Prefer clear commits that say **what** changed and **why**
- Append to `.cursor/changelog.md` (never rewrite); keep `.cursor/features.md` honest
- Ask Owl before inventing features, dependencies, or requirements that are not already decided
- Never commit Banter data dirs, `auth.json`, `client-session.json`, `voice.json`, `stream.json`, `home.json`, `dev-server.json`, or `.env`
- Do not force-push `main`; do not delete permanent platform branches

---

## Not in this build

Reactions, text chat inside a voice channel, group chats, kick, and ban remain deferred. See [`docs/foundation.md`](docs/foundation.md).
