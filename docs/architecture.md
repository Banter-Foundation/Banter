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
| `platform.rs` | Main GLFW window plus optional shared-context **Banter Settings** window; OpenGL 3.3, Dear ImGui docking + glow renderer (separate ImGui context per OS window via suspend/activate), clipboard, input, embedded-server bootstrap; UI present rate follows the current monitor (VSync `Sync(1)`, reapplied when the window moves or the mode Hz changes) |
| `ui.rs` | Signed-in Discord-like shell, sign-in/register, modals, chat, voice stage, 1:1 DM call stage, friends/DMs; `draw_main` vs `draw_settings_window` |
| `theme.rs` | Skin metrics and colors (temporary Discord dark theme) |
| `skin.rs` | Shared Dear ImGui draw helpers (buttons, inputs, menus, tooltips, …) |
| `settings.rs` | User Settings rules and `SettingsState` (OS-window host desire, layout, mic test, keybinds) — kept drawable-free for unit tests |
| `profile.rs` | Discord-parity profile card + customization editor state (`ProfileCard`, cosmetics drafts, board tabs, media drafts, editor slots) plus view/editor layout contracts (`profile_view_layout`, `profile_editor_layout`) — drawable-free for unit tests |
| `profile_paint.rs` | Procedural Banter-owned paint for nameplates, decorations, frames, effects, badges, and board game cards |
| `api.rs` | HTTP client (`ureq`) for `/v1/*` |
| `realtime.rs` | WebSocket client for presence, chat events, and WebRTC signaling |
| `session.rs` | Load/save/clear `client-session.json`; `persist_login` honors Stay signed in |
| `media.rs` (+ `media/encode.rs`, `rtc.rs`, …) | Mic/speakers (cpal), camera (nokhwa), screen share (xcap), I420→H.264 encode, WebRTC publish/subscribe |
| `logic.rs` | Pure helpers (key held, Dev Server prefs / URL normalize, etc.) |
| `icons.rs` | Lucide SVG → texture |
| `session` / prefs | Voice, stream, and `dev-server.json` live under the data dir, not in git |

Flow: `platform::run` loads `dev-server.json` and the saved session token, applies `startup_local_policy` (skip local for Remote; start local only when logged in on Local; otherwise reuse a running loopback or ask for remote), opens the window, builds `ui::App` against the chosen API base, then frames ImGui each tick. When User Settings is open, a second GLFW window titled **Banter Settings** is created (shared GL context) with its **own Dear ImGui context** (main is suspended for that pass) so input is not shared with the shell. Closing that window (chrome, Escape, or the in-UI X) clears settings state and focuses the main window. Developer settings (and the sign-in Server options overlay) can reconnect the client mid-session and start local when the user picks Local.

The signed-in layout is a **fixed column shell** (server rail, sidebar, stage, members). Panels are not treated as free-floating docks for day-to-day use even though docking is enabled and `banter-dock.ini` is written.

## Server (`banter-server`)

| Module | Responsibility |
| --- | --- |
| `main.rs` | Resolve `listen_addr`, print data dir, `serve` |
| `lib.rs` | `DEFAULT_ADDR`, `listen_addr`, `default_data_dir` / `resolve_data_dir`, re-exports |
| `http.rs` | Axum router: auth, friends, servers, channels, messages, DMs/calls, voice join/leave/flags, profile customization, `/v1/ws` |
| `store.rs` | PostgreSQL persistence (`schema.sql` via sqlx), including profile cosmetics and board widgets |
| `cosmetics.rs` | Banter-owned cosmetics catalog (nameplates, decorations, display-name styles, effects, frames) |
| `badges.rs` | Banter-owned profile badge catalog (shapes/colors for the Discord-style badge row) |
| `live.rs` | In-memory presence, voice membership, calls, WebSocket fan-out |
| `model.rs` | Shared types (accounts, channels, messages, voice flags, profile widgets, …) |
| `codec.rs` | Shared Opus/H.264 SDP contract (profile, RIDs, STUN/TURN helpers) |
| `error.rs` | `BanterError` → HTTP responses |

### Persistence vs memory

- **Survives restart:** accounts, friend graph, servers, invites, nicknames, categories, channels, messages, attachment metadata, banners, profile customization, and per-user server-rail folders in PostgreSQL; file bytes under `BANTER_DATA`.
- **Resets when the server process stops:** voice channel membership, 1:1 call state, presence, open WebSockets.

### HTTP surface (high level)

Auth and profile: `/v1/register`, `/v1/login`, `/v1/logout`, `/v1/me`, `/v1/users/{id}` (optional `server_id` for nickname), `/v1/me/profile` (customization patch including badges/activity/wishlist), `/v1/me/banner`, `/v1/users/{id}/banner`, `/v1/profile/cosmetics`, `/v1/profile/badges`, avatar upload/delete.  
Presence (`/v1/presence` and WS `presence` events) includes `custom_status` alongside online/idle/offline.  
Social: friends requests/list, presence, mentions.  
Servers: create/list, invites, members, nickname, categories, channels, layout.  
Server rail folders (per user): `/v1/me/server-rail`, `/v1/me/server-folders`, `/v1/me/servers/{id}/folder`.  
Chat: channel/DM messages, send/edit/delete, attachments. Message `body` is BFMD source (plain text valid); client parses/renders styled runs (`bfmd` / `bfmd_paint`). Spec: [bfmd.md](bfmd.md).  
Calls: DM call start/accept/decline/end.  
Voice: join, leave, flags.  
Realtime: `/v1/ws`.  
Health: `/v1/health`.

### Message threading (Reddit parity)

- Each message may set `reply_to` to another message in the **same** channel or DM (`messages.reply_to`, indexed). That is the parent pointer for nested comment trees.
- The API still returns a flat list ordered by id. The client builds a depth-first tree (`comment_tree_rows` in `logic.rs`): roots first, children nested under parents, orphans (missing parent) treated as roots.
- Collapse/expand is **client-local** (not persisted). Deleting a parent still clears children's `reply_to` (existing contract); those messages become roots.
- UI: bottom composer posts top-level only; **Reply** (toolbar or always-visible text link) opens an accented inline composer under that comment. Indent is 24 px per depth with thin vertical thread gutters (no L-hooks), circular (−)/(+) fold controls, and line-click collapse; leaf rows keep Discord-like avatar / time / hover toolbar polish.

Contract tests live in `crates/banter-server/tests/foundation.rs`. Client tree helpers are covered in `crates/banter/tests/client.rs`.

## Voice, camera, and stream (high level)

- Joining a voice channel (or starting a DM call) registers membership on the server and opens media paths on the client.
- Voice-channel SFU forward stamps each packet with the forward track’s negotiated SSRC/payload type and strips inbound RTP header extensions before `write_rtp` (publisher SSRCs are not valid on those senders).
- **Mute / deafen / push-to-talk** are client flags also pushed to the server; PTT defaults to **V** (ignored while a text field is focused) plus a Talk button. F1–F12 are available as keybinds.
- **Camera:** local preview is mirrored RGBA; outbound is I420 → H.264 over WebRTC (not JPEG spray). Open prefers ~`1280x720` (or nearest available), FPS near 30, device MJPEG on ties — not the device’s absolute highest mode. Device MJPEG is only the webcam capture format.
- **Screen share:** choose Screens or Applications, set resolution and FPS independently (default **720p / 30**), then **Go Live**. Nothing is sent until then. Encoder is **H.264 Constrained Baseline** (hardware when probed, else OpenH264).
- Mid-stream changes (source, size, FPS, bitrate) **pre-warm** on a side path and cut over on a frame boundary after an encoded keyframe; viewers resize without tearing the stream identity. Stream audio can mix mic, speaker loopback, and (on Windows) per-app sources with volumes. The Stream Select checklist groups them under **Audio channels** (system devices) and **Apps** (process loopback).
- Voice & stream prefs restore from `voice.json` / `stream.json`. Home landing layout restores from `home.json`. Developer Local/Remote restores from `dev-server.json`.
- Text chat **inside** a voice channel is deferred; the server rejects that path until Owl says otherwise.

## Modding

Client mods, server plugins, and external bots follow [modding/modding.md](modding/modding.md), with [mods](modding/mods.md), [plugins](modding/plugins.md), and [bots](modding/bots.md). The client runs enabled mods in Settings order. The server installs plugin zips and bot accounts, and a missing bot intent returns JSON null.

## Auth note (not finished)

Password + unique username work locally. Email, 2FA, Steam/Discord/Google OAuth, and `auth.json` secrets are designed but not the day-to-day path yet. OAuth redirects must not use raw IPs; Steam still needs a public http(s) hostname when that work resumes.
