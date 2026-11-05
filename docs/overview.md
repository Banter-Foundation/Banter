# Banter overview

Banter is a local desktop chat app with a Discord-like layout (temporary skin). It is meant as a foundation you can grow toward TeamSpeak- and Matrix-inspired ideas later, without waiting on a hosted service for day-to-day development.

## Pieces

| Piece | Crate / binary | Role |
| --- | --- | --- |
| Desktop client | `banter` → `banter.exe` | GLFW window, OpenGL 3.3, Dear ImGui docking UI, media capture, HTTP + WebSocket client |
| Local server | `banter-server` | Accounts, servers, channels, messages in PostgreSQL; files under `BANTER_DATA`; voice/calls/presence in memory; Axum HTTP + WebSocket |

The client talks to `http://127.0.0.1:3970` by default (Developer → Local). Startup does **not** require a local server unless you are already logged in on Local (it then tries to start the embedded server) or you choose Local in Developer settings. A saved Remote is used as-is with no local process. Fresh Local with nothing listening opens the UI and asks for a remote host instead of failing hard. User Settings → Developer (and the sign-in gear) switch Local vs Remote (`host`, `host:port`, or `http(s)://…`); the choice is saved in `dev-server.json` under the data directory.

## Stack

- **Language:** Rust (edition 2021). Workspace members: `crates/banter`, `crates/banter-server`.
- **Window / GL:** `glfw` 0.63, OpenGL **3.3** core profile, `glow` 0.18.
- **UI:** Dear ImGui **docking** branch via `dear-imgui-rs` 0.22 and `dear-imgui-glow` 0.22. `ConfigFlags::DOCKING_ENABLE` is set before the first frame.
- **Server HTTP:** Axum 0.8 (REST + `/v1/ws`), Tokio, sqlx (PostgreSQL).
- **Client media:** `cpal` (mic/speakers), `nokhwa` (camera), `xcap` (screen/window capture). WASAPI loopback / per-app audio are Windows-only.
- **Icons:** Lucide SVGs under `crates/banter/assets/lucide`, rasterized at runtime.

## Data paths

| OS | Default data directory |
| --- | --- |
| Windows | `%LOCALAPPDATA%\Banter` |
| Linux | `$XDG_DATA_HOME/Banter`, else `~/.local/share/Banter` |
| macOS | `~/Library/Application Support/Banter` |

`BANTER_DATA` overrides the default when it is non-empty.

Typical files under the data directory:

- PostgreSQL (`BANTER_DATABASE_URL`) — accounts, servers, channels, messages, attachment metadata
- `avatars/`, `files/` — blobs
- `client-session.json` — last login token when **Stay signed in** was checked (shared across clients on this machine; last writer wins on next launch). Unchecked sign-in leaves no file so relaunch asks for credentials again.
- `voice.json`, `stream.json` — Voice & Video and Go Live preferences
- `home.json` — customizable Home landing section order, visibility, and recent destinations
- `dev-server.json` — Developer Local vs Remote API target and remote host
- `banter-dock.ini` — Dear ImGui layout
- `auth.json` — reserved for future secrets; never commit it

Do not commit anything under the Banter data directory, nor `auth.json`, `client-session.json`, `voice.json`, `stream.json`, `home.json`, `dev-server.json`, or `.env`.

## Listen address

Constant used by the desktop client and as the fallback for the server binary:

```text
127.0.0.1:3970
```

(`banter_server::DEFAULT_ADDR`)

**`banter-server` binary** chooses the bind address with `listen_addr`:

1. Non-empty first CLI argument wins.
2. Else if `BANTER_BIND` and/or `BANTER_PORT` are set (non-empty), use `{bind or 0.0.0.0}:{port or 3970}`.
3. Else `127.0.0.1:3970`.

The **embedded** server started by the desktop app always uses `127.0.0.1:3970`. Container images set `BANTER_BIND=0.0.0.0` so the port is reachable from the host.

## Related docs

- [Build and run](build-and-run.md)
- [Architecture](architecture.md)
- [Feature status](feature-status.md)
- [Foundation / deferred list](foundation.md)
- [Contributing](contributing.md)
- [Current build notes](current-build.md) (what was observed on a live machine)
- Root [README](../README.md)
