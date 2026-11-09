# Banter

Local desktop chat. The window is GLFW and OpenGL 3.3. The interface is Dear ImGui from the docking branch (`dear-imgui-rs` 0.22). Accounts, servers, channels, and messages stay in PostgreSQL after the app closes; uploaded files live under `BANTER_DATA`. Voice membership, calls, and presence are in memory and reset when the server process stops.

**Docs for collaborators**

| Doc | Contents |
| --- | --- |
| [docs/overview.md](docs/overview.md) | What Banter is, stack, data paths, `127.0.0.1:3970` / `BANTER_BIND` / `BANTER_PORT` |
| [docs/build-and-run.md](docs/build-and-run.md) | Windows/Linux build & run, tests |
| [docs/docker-setup.md](docs/docker-setup.md) | Podman/Docker server image setup |
| [docs/architecture.md](docs/architecture.md) | Crates, modules, voice/camera/stream notes |
| [docs/errors.md](docs/errors.md) | Stable `BANTER_*` error codes, toast pattern, voice/media first |
| [docs/chat-commands.md](docs/chat-commands.md) | Composer slash/prefix commands, per-guild prefix + aliases, registry |
| [docs/version.md](docs/version.md) | Release label + protocol floors; client version checker |
| [docs/feature-status.md](docs/feature-status.md) | What works vs deferred (honest) |
| [docs/bfmd.md](docs/bfmd.md) | Chat text formatting language (incl. `{e}` emote size) |
| [docs/emote-library.md](docs/emote-library.md) | Emote / sticker / GIF packs, upload, picker (BA-83) |
| [docs/foundation.md](docs/foundation.md) | Chosen foundation slice + deferred list |
| [docs/contributing.md](docs/contributing.md) | Branches, worktrees, commits, changelog |
| [docs/current-build.md](docs/current-build.md) | Live observation notes from a recent docs pass |
| [docs/ui-theme-layout.md](docs/ui-theme-layout.md) | Theme/layout module boundaries and shell regions |
| [docs/modding/modding.md](docs/modding/modding.md) | Mods, plugins, and bots (specification; not loaded yet) |

## Run

### Windows

Use a Visual Studio developer shell so `cl.exe` is available (Dear ImGui compiles C/C++). PostgreSQL must be reachable (see [docs/docker-setup.md](docs/docker-setup.md)):

```
cd "D:\Projects\Project Banter"
cargo run -p banter
```

Data goes to `%LOCALAPPDATA%\Banter` unless `BANTER_DATA` is set. Keep the repo and `target/` under `D:\Projects\Project Banter`. Do not use `E:` for Banter builds.

### Linux

Install a C toolchain, GLFW/OpenGL, ALSA or PipeWire, V4L2 headers, and X11 (for the cursor overlay). On Arch/CachyOS:

```
sudo pacman -S --needed base-devel clang cmake pkgconf glfw mesa alsa-lib pipewire libx11
cargo run -p banter
```

Data goes to `$XDG_DATA_HOME/Banter`, or `~/.local/share/Banter`, unless `BANTER_DATA` is set. Speaker loopback and per-app stream audio stay Windows-only for now; microphone, speakers, camera, and screen share work.

### macOS (Apple Silicon)

Open **`Banter.app`** from the pre-release zip (double-click or `open Banter.app`). Releases ship the `.app` bundle — not a bare `banter-macos-arm` Mach-O — so Finder does not open Terminal. The first mic, camera, or screen share prompts macOS Privacy & Security; enable Banter under **Microphone**, **Camera**, and **Screen Recording**, then quit and reopen Banter. Data goes to `~/Library/Application Support/Banter`. Speaker loopback / per-app stream audio are not available on macOS yet. Opening http links from the client is not implemented.

### Server

By default the client uses Local mode (`127.0.0.1:3970`). If nothing is there, it starts the server in the background. **User Settings → Developer** switches between Local and a Remote host (`host`, `host:port`, or URL); the choice is saved in `dev-server.json` under the data directory.

To run the server on its own:

```
cargo run -p banter-server
```

An optional listen address can be passed as the first argument, and that argument wins. Otherwise `BANTER_BIND` and `BANTER_PORT` are used, with `0.0.0.0` and `3970` filling whichever one is missing. When the argument and both variables are absent, the server uses `127.0.0.1:3970`. The desktop app still starts its own server on `127.0.0.1:3970`.

## Server container

Setup guide: **[docs/docker-setup.md](docs/docker-setup.md)**. Podman and Docker both run this image. Compose builds `banter-server:local` from this checkout. The last image that landed on GHCR is `version-1-test-8`. `version-1-test-9` did not push.

```
cd "D:\Projects\Project Banter"
podman compose up -d --build
# or: docker compose up -d --build
```

Inside the container the server binds `0.0.0.0` on port `3970` (`BANTER_BIND` / `BANTER_PORT`). Account data stays on the named volume `banter-data`. Point the desktop client at a published port with **User Settings → Developer → Remote**.

Image: Compose builds `banter-server:local` from this checkout. `ghcr.io/snowedinowl/banter:version-1-test-8` is the last tag that landed, until a classic PAT with `write:packages` pushes a newer one. Desktop and server binaries for the current pre-release are [Version-1-test-13.4](https://github.com/Banter-Foundation/Banter/releases/tag/Version-1-test-13.4) (Linux assets in this cut; Win/macOS deferred).

Open the client twice to try voice between two accounts. They share one server. With **Stay signed in** checked, `client-session.json` stores one login for the next cold start; unchecked keeps the token in memory only for that run. Both clients can stay signed in during the same run because each keeps its token in memory.

## Window

Before the first frame the client sets Dear ImGui `ConfigFlags::DOCKING_ENABLE`. After login the main viewport is a Discord-like fixed column shell (temporary skin):

- left: server list, then the sidebar (friends or channels)
- right: members
- center: chat, with voice on the stage when you join a voice channel

You can drag those panels. The layout is kept in `banter-dock.ini` in the data folder.

Push-to-talk is the V key, and it is ignored while a text field has the keyboard. The Voice dock also has a Talk button you can hold. Mute, deafen, camera, and screen share are in that same dock. The camera opens at about 720p (`1280x720`) or the nearest available mode, preferring MJPEG so Facecam and Elgato keep working; it does not grab the device's highest resolution and then downscale. If the microphone, speakers, or camera cannot be opened, the voice-connection panel shows one plain error in the subtitle (User Settings still lists every device error). Deafen turns mute on and restores the previous mute when you turn deafen off.

F3 toggles the profiler: FPS, process memory, HTTP and WebSocket traffic, frame section timings, and a log of recent expensive operations. Escape closes it. Optional checkboxes open Dear ImGui Metrics and Demo. User Settings → Voice & Video → Video Settings controls VSync, fullscreen, and window resolution (`video.json` in the data folder). With VSync on, the UI follows the current monitor’s refresh rate.

Voice channels do not have a text box. Text chat inside a voice channel is deferred.

## Tests

```
cargo test -p banter-server
cargo test -p banter
```

## Not in this build

Reactions, text chat inside a voice channel, group chats, kick, and ban. About Me and custom status are on My Account and the profile. Guild roles are in the server menu. See [docs/foundation.md](docs/foundation.md) and [docs/feature-status.md](docs/feature-status.md).
