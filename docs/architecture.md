# Architecture

Two crates in one Cargo workspace (`Cargo.toml` at the repo root).

```text
Banter/
  crates/banter/           # desktop client
  crates/banter-server/    # local API + PostgreSQL + live state
  docs/
  Dockerfile               # server image only
  docker-compose.yml
```

## Client (`banter`)

| Module | Responsibility |
| --- | --- |
| `main.rs` | Entry → `banter::run()` |
| `platform.rs` | GLFW window, OpenGL 3.3 context, Dear ImGui docking + glow renderer, clipboard, input, embedded-server bootstrap |
| `ui.rs` | Signed-in Discord-like shell, sign-in/register, modals, chat, voice stage, 1:1 DM call stage, friends/DMs |
| `theme.rs` | Skin metrics and colors (temporary Discord dark theme) |
| `skin.rs` | Shared Dear ImGui draw helpers (buttons, inputs, menus, tooltips, …) |
| `settings.rs` | User Settings rules and `SettingsState` (layout, mic test, keybinds) — kept drawable-free for unit tests |
| `profile.rs` | Profile card rules (`ProfileCard`, headings, primary actions) — drawable-free for unit tests |
| `api.rs` | HTTP client (`ureq`) for `/v1/*` |
| `realtime.rs` | WebSocket client for presence, chat events, and WebRTC signaling |
| `session.rs` | Load/save `client-session.json` |
| `media.rs` (+ `media/encode.rs`, `rtc.rs`, …) | Mic/speakers (cpal), camera (nokhwa), screen share (xcap), I420→H.264 encode, WebRTC publish/subscribe |
| `logic.rs` | Pure helpers (key held, Dev Server prefs / URL normalize, etc.) |
| `icons.rs` | Lucide SVG → texture |
| `session` / prefs | Voice, stream, and `dev-server.json` live under the data dir, not in git |

Flow: `platform::run` loads `dev-server.json`, ensures a local server only when the target is Local, opens the window, builds `ui::App` against the chosen API base, then frames ImGui each tick. Developer settings can reconnect the client mid-session.

The signed-in layout is a **fixed column shell** (server rail, sidebar, stage, members). Panels are not treated as free-floating docks for day-to-day use even though docking is enabled and `banter-dock.ini` is written.

## Server (`banter-server`)

| Module | Responsibility |
| --- | --- |
| `main.rs` | Resolve `listen_addr`, print data dir, `serve` |
| `lib.rs` | `DEFAULT_ADDR`, `listen_addr`, `default_data_dir` / `resolve_data_dir`, re-exports |
| `http.rs` | Axum router: auth, friends, servers, channels, messages, DMs/calls, voice join/leave/flags, `/v1/ws` |
| `store.rs` | PostgreSQL persistence (`schema.sql` via sqlx) |
| `live.rs` | In-memory presence, voice membership, calls, WebSocket fan-out |
| `model.rs` | Shared types (accounts, channels, messages, voice flags, …) |
| `codec.rs` | Shared Opus/H.264 SDP contract (profile, RIDs, STUN/TURN helpers) |
| `error.rs` | `BanterError` → HTTP responses |

### Persistence vs memory

- **Survives restart:** accounts, friend graph, servers, invites, nicknames, categories, channels, messages, attachment metadata in PostgreSQL; avatars and file bytes under `BANTER_DATA`.
- **Resets when the server process stops:** voice channel membership, 1:1 call state, presence, open WebSockets.

### HTTP surface (high level)

Auth and profile: `/v1/register`, `/v1/login`, `/v1/logout`, `/v1/me`, `/v1/users/{id}` (optional `server_id` for nickname), avatar upload/delete.  
Social: friends requests/list, presence, mentions.  
Servers: create/list, invites, members, nickname, categories, channels, layout.  
Chat: channel/DM messages, send/edit/delete, attachments.  
Calls: DM call start/accept/decline/end.  
Voice: join, leave, flags.  
Realtime: `/v1/ws`.  
Health: `/v1/health`.

Contract tests live in `crates/banter-server/tests/foundation.rs`.

## Voice, camera, and stream (high level)

- Joining a voice channel (or starting a DM call) registers membership on the server and opens media paths on the client.
- **Mute / deafen / push-to-talk** are client flags also pushed to the server; PTT defaults to **V** (ignored while a text field is focused) plus a Talk button. F1–F12 are available as keybinds.
- **Camera:** local preview is mirrored RGBA; outbound is I420 → H.264 over WebRTC (not JPEG spray). Open prefers ~`1280x720` (or nearest available), FPS near 30, device MJPEG on ties — not the device’s absolute highest mode. Device MJPEG is only the webcam capture format.
- **Screen share:** choose Screens or Applications, set resolution and FPS independently (default **720p / 30**), then **Go Live**. Nothing is sent until then. Encoder is **H.264 Constrained Baseline** (hardware when probed, else OpenH264).
- Mid-stream changes (source, size, FPS, bitrate) **pre-warm** on a side path and cut over on a frame boundary after an encoded keyframe; viewers resize without tearing the stream identity. Stream audio can mix mic, speaker loopback, and (on Windows) per-app sources with volumes.
- Voice & stream prefs restore from `voice.json` / `stream.json`. Developer Local/Remote restores from `dev-server.json`.
- Text chat **inside** a voice channel is deferred; the server rejects that path until Owl says otherwise.

## Auth note (not finished)

Password + unique username work locally. Email, 2FA, Steam/Discord/Google OAuth, and `auth.json` secrets are designed but not the day-to-day path yet. OAuth redirects must not use raw IPs; Steam still needs a public http(s) hostname when that work resumes.
