# Banter

[![CI](https://github.com/Banter-Foundation/Banter/actions/workflows/ci.yml/badge.svg)](https://github.com/Banter-Foundation/Banter/actions/workflows/ci.yml)
[![Latest release](https://img.shields.io/github/v/release/Banter-Foundation/Banter?include_prereleases)](https://github.com/Banter-Foundation/Banter/releases)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

**Banter** is a local-first desktop chat and voice app. Sign in, create or join servers, talk in text and voice channels, call friends over DMs, and share your camera or screen — on a stack you can run yourself.

It ships as two pieces: a native desktop client (`banter`) and a local API server (`banter-server`). Chat history and accounts live in **PostgreSQL**; uploaded files stay under `BANTER_DATA`. Voice membership, calls, and presence are in memory and reset when the server process stops.

The UI currently uses a Discord-like layout as a temporary skin (not final Banter branding; no Discord logos). Longer term, Banter is meant to grow toward TeamSpeak- and Matrix-inspired ideas on the same local-first base. What ships today is the **foundation** — see [Feature status](docs/feature-status.md).

## Features

- Local username + password accounts, display name, and avatar
- Servers, invites, member list, server nicknames, and guild roles
- Text and voice channels in categories (plus drawing-board channels)
- Friends, 1:1 DMs, and DM calls
- Voice: mute, deafen, push-to-talk, camera, and screen share (Go Live)
- BFMD chat formatting (not CommonMark) — [docs/bfmd.md](docs/bfmd.md)
- Profiles, threaded replies, emote library, slash/prefix commands, compose autosuggest
- Local (`127.0.0.1:3970`) or Remote API target in **User Settings → Developer**

Honest working vs deferred list: [docs/feature-status.md](docs/feature-status.md) · [docs/foundation.md](docs/foundation.md).

## Supported platforms

| Platform | Client | Notes |
| --- | --- | --- |
| Windows | `banter.exe` | Full desktop target |
| Linux | `banter` | X11 / PipeWire-ready; speaker loopback / per-app stream audio stay Windows-only |
| macOS (Apple Silicon) | `Banter.app` | Grant Microphone, Camera, and Screen Recording; loopback / per-app audio not available |

Mainline in this repo is the **desktop** client + server. Android and website work live on permanent platform branches — [docs/permanent-branches.md](docs/permanent-branches.md). There is no App Store distribution path documented here.

## Download

Pre-releases are tagged `Version-1-test-N` on [GitHub Releases](https://github.com/Banter-Foundation/Banter/releases).

| Release | What’s attached |
| --- | --- |
| **[Version-1-test-13.4](https://github.com/Banter-Foundation/Banter/releases/tag/Version-1-test-13.4)** (current) | Linux client zip + `banter-server-linux`; macOS Apple Silicon `Banter.app` zip + `banter-server-macos-arm` |
| **[Version-1-test-13.3](https://github.com/Banter-Foundation/Banter/releases/tag/Version-1-test-13.3)** | Full cut including Windows client zip + `banter-server.exe` |

Always check the release page for the exact asset list — some cuts defer a platform.

**macOS:** open **`Banter.app`** from the zip (`open Banter.app` or double-click). Do not run a bare Mach-O (Finder would open Terminal). After the first privacy prompt, enable Banter under **System Settings → Privacy & Security**, then quit and reopen.

## Quick start

1. Install a pre-release build (above) **or** [build from source](#building-from-source).
2. Start **PostgreSQL** (required for Local mode). Easiest from this repo:

   ```bash
   podman compose up -d postgres
   # or: docker compose up -d postgres
   ```

   Default URL: `postgres://banter:banter@127.0.0.1:5432/banter` (`BANTER_DATABASE_URL`). See [docs/build-and-run.md](docs/build-and-run.md) if port `5432` is already taken.
3. Launch the desktop app. **User Settings → Developer → Local** uses `http://127.0.0.1:3970`. Choose **Remote** to point at another host.
4. Register an account and create or join a server.

Data directory (override with `BANTER_DATA`):

| OS | Path |
| --- | --- |
| Windows | `%LOCALAPPDATA%\Banter` |
| Linux | `$XDG_DATA_HOME/Banter` or `~/.local/share/Banter` |
| macOS | `~/Library/Application Support/Banter` |

## Building from source

### Prerequisites

- **Rust** stable (built with 1.99.x)
- **C/C++ toolchain** (Dear ImGui needs a native compiler — Visual Studio Build Tools / `cl.exe` on Windows; `base-devel` / clang on Linux)
- **PostgreSQL 16** reachable at `BANTER_DATABASE_URL`
- A display (GLFW opens a real window)

Linux (Arch / CachyOS example):

```bash
sudo pacman -S --needed base-devel clang cmake pkgconf glfw mesa alsa-lib pipewire libx11
```

### Clone and build

```bash
git clone https://github.com/Banter-Foundation/Banter.git
cd Banter

# Optional: copy env defaults
cp .env.example .env

podman compose up -d postgres   # or: docker compose up -d postgres

cargo build -p banter
cargo build -p banter-server
```

Debug binaries land in `target/debug/` (`banter.exe` / `banter-server.exe` on Windows).

Known checkout path on Owl’s Windows machine: `D:\Projects\Project Banter` (keep `target/` there; do not use `E:`).

Full platform notes: [docs/build-and-run.md](docs/build-and-run.md).

### Run

```bash
# Desktop client (day-to-day)
cargo run -p banter

# Server alone
cargo run -p banter-server
# Optional bind (wins over env):
# cargo run -p banter-server -- 0.0.0.0:3970

curl -s http://127.0.0.1:3970/v1/health
# Expect ok, version, protocol, min_client_protocol — docs/version.md
```

Listen address for `banter-server`: first CLI argument, else `BANTER_BIND` / `BANTER_PORT`, else `127.0.0.1:3970`. The embedded server started by the desktop app always uses `127.0.0.1:3970`. Details: [docs/overview.md](docs/overview.md).

To try voice between two accounts on one machine, run the client twice. With **Stay signed in** checked, `client-session.json` stores one token (last writer wins on next cold start). Point `BANTER_DATA` at an empty folder for a fresh login without logging out the other window.

### Server container

Compose runs Postgres + Caddy + blue/green `banter-a` / `banter-b`. The image is **`banter-server` only** (not the desktop app).

```bash
podman compose up -d --build
# or: docker compose up -d --build
```

Default image: `banter-server:local` from this checkout. Last published GHCR tag: `ghcr.io/snowedinowl/banter:version-1-test-8` (newer pushes blocked — see [docs/docker-setup.md](docs/docker-setup.md)). Point the desktop client at the published host port with **Developer → Remote**.

## Project layout

```text
Banter/
  crates/banter/         # desktop client
  crates/banter-server/  # API + PostgreSQL + live state
  testing-debug/         # headless Linux stream debug CLI
  docs/                  # guides
  deploy/                # Caddyfile + active slot for hot-swap
  scripts/               # packaging, deploy-hot-swap, checks
  Dockerfile
  docker-compose.yml
```

Module map: [docs/architecture.md](docs/architecture.md).

## Documentation

| Doc | Contents |
| --- | --- |
| [docs/overview.md](docs/overview.md) | Product, stack, data paths, bind address |
| [docs/build-and-run.md](docs/build-and-run.md) | Windows / Linux / macOS build & run |
| [docs/docker-setup.md](docs/docker-setup.md) | Podman / Docker, hot-swap, GHCR status |
| [docs/architecture.md](docs/architecture.md) | Crates and modules |
| [docs/feature-status.md](docs/feature-status.md) | What works vs deferred |
| [docs/foundation.md](docs/foundation.md) | Foundation slice + deferred list |
| [docs/version.md](docs/version.md) | Release labels + protocol floors |
| [docs/errors.md](docs/errors.md) | Stable `BANTER_*` codes |
| [docs/bfmd.md](docs/bfmd.md) | Chat text formatting |
| [docs/emote-library.md](docs/emote-library.md) | Emote / sticker / GIF packs |
| [docs/chat-commands.md](docs/chat-commands.md) | Composer commands |
| [docs/streaming.md](docs/streaming.md) | Voice / stream media |
| [docs/contributing.md](docs/contributing.md) | Branches, commits, changelog |
| [docs/modding/modding.md](docs/modding/modding.md) | Mods / plugins / bots (spec only) |

## Development

```bash
cargo test -p banter-server   # needs Postgres at BANTER_DATABASE_URL
cargo test -p banter
```

Do not weaken `crates/banter-server/tests/foundation.rs` — that suite is the API contract.

CI ([`.github/workflows/ci.yml`](.github/workflows/ci.yml)):

- **Linux** — full `banter-server` / `banter` suites against `postgres:16`, plus a macOS `.app` zip layout check
- **Windows** — `--lib` unit tests only (no Postgres on that job)

## Contributing

See [docs/contributing.md](docs/contributing.md). In short:

- Prefer commits that say **what** changed and **why**
- Keep docs in `docs/` and `.cursor/changelog.md` honest when behavior changes
- Never commit Banter data dirs, `auth.json`, `client-session.json`, `voice.json`, `stream.json`, `home.json`, `dev-server.json`, or `.env`
- Do not force-push `main`; do not delete permanent platform branches (`linux`, `windows`, `mac`, `android`, `website`)

## Status notes

- Composer file attach may show **Currently Unavailable**; existing attachments still download
- Voice / presence reset when the server process stops; Postgres chat history does not
- Reactions, text chat inside a voice channel, group chats, kick, and ban are deferred — [docs/foundation.md](docs/foundation.md)

## License

The `banter` and `banter-server` crates are licensed under **MIT OR Apache-2.0** (see each crate’s `Cargo.toml`).
