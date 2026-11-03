## 2026-11-02 22:51:20 -06:00 [Winter] — Banter desktop foundation

The local server keeps accounts, servers, channels, messages, and files in SQLite. The desktop client is a GLFW OpenGL window using Dear ImGui's docking branch. Voice, calls, and presence reset when the server process stops.

- Accounts, friend requests, servers, invites, nicknames, categories, text and voice channels
- Messages: send, reply, edit, delete, attach, search, mentions
- Drop-in voice and 1:1 calls with mute, deafen, push-to-talk, camera, and screen share
- Docked window: servers, sidebar, members, chat, and voice
- Deferred list kept in `docs/foundation.md` and not implemented

## 2026-11-02 23:04:07 -06:00 [Winter] — Show password

The sign-in password stays hidden as `*` until Show password is checked.

## 2026-11-02 23:12:08 -06:00 [Winter] — Copy and paste

Text fields use the system clipboard. A hidden password still draws `*`, and Ctrl+C / Ctrl+V stay available.

## 2026-11-02 23:58:14 -06:00 [Winter] — Discord-style layout

The signed-in window is a fixed column shell: server rail, channel sidebar, stage, and member list. Voice controls sit at the bottom of the sidebar. Clicking a voice channel joins it. Text chat inside a voice channel stays deferred.

## 2026-11-03 00:14:49 -06:00 [Winter] — Temporary Discord screen

The signed-in window now follows Discord's colors, server icons, channel rows, chat header, member groups, and the voice and user bars. This skin is temporary and does not use Discord's logo. Server settings stay in the channel-list menu.

## 2026-11-03 01:14:51 -06:00 [Winter] — Settings, camera, and screen share

The signed-in window uses Lucide icons. User Settings has My Account and Voice & Video. Camera preview is mirrored and follows the selected camera. Screen share waits for Go Live, with Screens and Applications plus 480p, 720p, 1080p, Source and 15, 30, or 60 FPS. A voice channel shows participant tiles. Client tests: 20 passed. Built and launched from `D:\Projects\Banter\target\debug\banter.exe`.

## 2026-11-03 02:27:17 -06:00 [Winter] — Voice activity, stream quality, and live switching

Voice settings now drive the capture engine and are saved in the data directory. A second client receives microphone audio. The green speaking ring follows the mic level. Stream resolution and frame rate are independent, and a mid-stream change waits for a ready keyframe on a frame boundary.

- Voice & Video: input and output devices, volume, Let's Check meter, voice activity or push to talk, sensitivity, keybind, release delay, noise suppression, echo cancellation, and automatic gain control
- Speaking ring uses the measured mic level in voice activity, and the held key in push to talk
- Go Live keeps Screens and Applications. Resolution is 480p, 720p, 1080p, 1440p, 4K, or Source. Frame rate is 15, 30, 60, 120, or 144. A choice the monitor cannot do stays visible with the reason
- The upward chevron by screen share sets software JPEG, bitrate, cursor, and your own preview. Hardware H.264 stays unavailable on this PC and says so
- Stream audio is a checklist of microphones, speaker loopback, and application processes, each with a volume. Last-used choices restore on the next launch
- A live change of source, size, frame rate, or bitrate keeps the same stream and cuts over on a frame boundary after the next picture is encoded. Viewers resize the picture while it keeps playing
- Client tests: 24 passed, including a 1920x1080 encode and a second client that received audio and a quality switch with no dropped frames inside one frame interval. Server tests: 10 passed

## 2026-11-03 02:34:57 -06:00 [Winter] — Docker files, not built

Uncommitted `Dockerfile`, `docker-compose.yml`, and `.dockerignore` are in the repo root. Docker Desktop is not installed on this machine, so the image was not built, compose was not run, and these files were not committed. `banter-server` still takes its listen address from argv or `127.0.0.1:3970`. The desktop default is unchanged.

## 2026-11-03 02:37:44 -06:00 [Winter] — Server listen address from the environment

`banter-server` now chooses its listen address with `listen_addr`. A non-empty command-line argument wins. Otherwise `BANTER_BIND` and `BANTER_PORT` select the address, using `0.0.0.0` or `3970` when only one of them is set. When the argument and both variables are absent, the address stays `127.0.0.1:3970`. The desktop app still starts its embedded server on that default. `listen_addr_prefers_the_argument_then_env_then_loopback` passed with the other 10 server foundation tests. Docker files stay uncommitted. No tag and no release.

## 2026-11-03 10:34:06 -06:00 [Winter] — Desktop window open, Docker Desktop engine down

The debug app is open so it can be looked at: pid 15564, title Banter, responding. That exe was built at 02:24 and is newer than the UI sources. Docker Desktop's engine pipe is missing, so the image was not built and the pre-release was not created. `write:packages` is on the gh token. The default `docker` server is Podman, not Docker Desktop.

## 2026-11-03 10:44:32 -06:00 [Winter] — Podman server image

The server image `ghcr.io/snowedinowl/banter:version-1-test-1` was built with Podman and pushed to GHCR. Compose starts it with `BANTER_BIND=0.0.0.0`. The desktop app still defaults to `127.0.0.1:3970`. A register and a text message succeeded against the container. The Dockerfile gives the workspace a stub desktop crate so Cargo can load the manifest without compiling the app.

## 2026-11-03 11:04:39 -06:00 [Winter] — Chat rows and profile pictures

Text rows show a 40px round avatar, a bold display name, and a muted local time such as "Today at 10:17 AM". The full date is on hover. Messages from the same person within 7 minutes on the same local day are grouped, with the time in the gutter on hover. A date change draws a separator. The row highlights on hover, and the toolbar (reply, edit, delete, More) only shows then. More copies the message text.

My Account can upload a PNG, JPG, GIF, or WebP, crop and zoom it into a circle, and save or remove it. The server stores a PNG no larger than 512px on a side and rejects files over 2 MB. Removing it returns the letter avatar. Chat rows, the member list, voice tiles, the user bar, and direct messages use the saved picture. A revision on the account is polled every 2 seconds so another client updates without a restart.

The settings audit was not started in this tree. The only settings overlap is the avatar controls on My Account. `D:\Projects\Banter-settings` and the `settings-fixes` branch were not edited.

Client tests: 26 passed. Server foundation tests: 12 passed. Debug window opened at 11:04:05 AM America/Chicago, pid 34512, title Banter. The `Version-1-test-1` tag was not moved. Pre-release: https://github.com/SnowedInOwl/Banter/releases/tag/Version-1-test-1. Image: `ghcr.io/snowedinowl/banter:version-1-test-1`. Container commit: https://github.com/SnowedInOwl/Banter/commit/e134bc0. Attachments: `Banter-windows-version-1-test-1.zip` and `banter-server.exe`.
## 2026-11-03 11:36:08 -06:00 [Winter] — Discord skin pass on ui-polish

Worktree `D:\Projects\Banter-ui`, branch `ui-polish`. The signed-in window and sign-in screens are redrawn to Discord's desktop metrics from one set of constants in `crates/banter/src/theme.rs`, with shared drawing helpers in `crates/banter/src/skin.rs`. Chat message rows, avatar upload, and User Settings were left to the agents that own them.

- Sign-in and register: blurple gradient, 480 px card with a soft shadow, uppercase field labels with the error in the label, `*` mask and Show password kept, 44 px Log In / Continue, Register and login links
- Server rail: white pill (8 px hover, 40 px selected), circle-to-squircle on hover and select, 32x2 separator, green add-server button, right-side tooltip with an arrow, red mention badge
- Channel list: server header with chevron or X and a floating dropdown (Invite People, Create Channel, Create Category for the owner, Change Nickname), collapsible categories with chevrons and an owner-only plus, 32 px channel rows, voice participants under the channel, Voice Connected panel with signal icon, user panel
- Chat header: channel icon and name, members toggle, search box, phone icon to start a DM call. Composer: 44 px rounded bar with upload button and "Message #channel" placeholder
- Members: uppercase role headers with counts, 42 px rows, Discord status dots (cut-out ring, idle crescent, offline ring), dimmed offline rows, owner crown with a "Server Owner" tooltip
- Voice and video: black stage, 16:9 tile grid with 8 px gaps, green speaking border, name tags, 56 px round call controls with a split screen-share button and red disconnect, Join Voice empty state
- Friends and DMs: tabs with pending badge, Add Friend box, 62 px people rows with round actions, DIRECT MESSAGES label
- Modals: Add a Server, Create Server, Join Server, Invite (Copy turns green), Change Nickname, Create Channel (type cards), Create Category, all with Discord's overlay, radius, padding and footer
- Voice lists and tiles show your display name instead of "You"
- Removed unused `add_channel`, `rail_icon`/`rail_gap`/`server_fill` free functions, and the old inline create-channel form and `draw_call`; the modals replace them
- Tests: `crates/banter/tests/theme.rs` written first, 14 passed. Client 24 passed, server 11 passed

## 2026-11-03 11:46:07 -06:00 [Winter] — Chat rows merged into ui-polish, history sits on the composer

`main` (`fa4b1e7`: 40 px chat rows, grouping, hover toolbar, profile pictures) is merged into `ui-polish`. Conflicts kept the ui-polish layout and added main's behavior: Escape also closes the avatar crop and the More menu, the user-panel avatar opens My Account, and the `ellipsis` icon is in the table. Main's second `dm_row` was dropped because the ui-polish one already draws the avatar and status.

- The message history is padded from the top so a short channel sits on the composer, as in Discord. It follows new messages only when already at the bottom, and always jumps after you send
- Every message row now ends at its own bottom edge. A hovered row's toolbar no longer shifts the rows below it, and a history that fits no longer shows a scrollbar
- Rows were compared pixel by pixel with `media/discord-message-row-reference.png` and `media/discord-message-row-hover.png`: name, timestamp, body, toolbar, and colors line up within 1 px
- Tests: `a_short_history_is_pushed_to_the_bottom_of_the_pane` and `chat_follows_new_messages_only_from_the_bottom_or_after_sending` written first. Theme 16, client 26, server 12 passed

## 2026-11-03 12:06:16 -06:00 [Winter] — Merge settings-fixes into ui-polish

User Settings from `settings-fixes` (`ae0395c`) is merged into `ui-polish` by hand: the Discord shell skin stays, and Settings uses `SettingsState`, the centered layout, Escape that closes dropdowns first, the rebuilt Voice & Video controls, and the camera MJPEG fix.

- Added `crates/banter/src/settings.rs` and its tests (settings, device errors, slider, camera frames)
- `media.rs` takes the settings camera path: MJPEG decode, one error per failure run, clear the preview when switching cameras
- `VoicePrefs` keeps a camera name and `#[serde(default)]` so older `voice.json` / `stream.json` still load
- My Account keeps crop-and-zoom upload and Remove Avatar from main
- Platform maps F1–F12 for push-to-talk keybinds
- Workspace tests: client 26, theme 16, settings 18, camera 4, device errors 6, slider 4, server 12 — all passed

## 2026-11-03 12:17:12 -06:00 [Winter] — Native Linux desktop

Banter builds and runs on Linux (CachyOS / KDE Wayland). The Windows path is unchanged.

- Data directory uses `$XDG_DATA_HOME/Banter` or `~/.local/share/Banter` when `BANTER_DATA` is unset; `BANTER_DATA` still wins on every OS
- Fonts load Liberation Sans, then Noto Sans, then DejaVu Sans (Segoe UI stays on Windows)
- Screen-share cursor overlay uses X11 `XQueryPointer` (works on XWayland); pure Wayland skips the stamp
- `xcap` bumped to 0.9 so PipeWire 1.6 / LLVM 22 bindgen works (`libspa` 0.10)
- Speaker loopback and per-app stream audio stay Windows-only and still say so
- README covers Linux packages; design notes list the Linux font fallbacks
- Client live-quality test keeps the 16 ms logical gap check and only requires wall delivery to finish in under 800 ms
- Tests on this machine: server foundation 13, client 26, theme 16, settings 18, camera 4, slider 4 — all passed. Debug `banter` stayed up, `/v1/health` was ok, and register against the embedded server succeeded
