## 2026-11-07 01:27:06 -06:00 [ubuntu] — Emote Library vertical slice (BA-83)

Ships packs/assets/subscriptions schema (`007_emote_library`), upload with Carbon size caps and locked 15s GIF duration (classic + video), personal subscribe + server packs, BFMD `{e}` emote sizing (default 22px), stable `:{e:id}:` tokens, Model A sticker send, GIF message media, and composer Emotes/Stickers/GIFs picker. No mock packs; Unicode emoji keyboard and reactions stay deferred. Docs: `docs/emote-library.md`.

## 2026-11-07 07:13:31 +00:00 [ubuntu] — Chat slash command system (BA-104)

Discord-like composer commands with a configurable per-guild prefix (not hard-coded `/`), alias map, popup (Tab/Enter), and `CommandRegistry` for later plugins. Built-ins: `me`, `shrug`/`shruggie`, `tableflip`, `mute`, `deafen`, `join`, stub `nick`, hidden `debugnow`. Server persists prefix + aliases (`007_chat_commands`); Chat Commands in the server menu. Docs: `docs/chat-commands.md`.

## 2026-11-07 01:20:00 -06:00 [ubuntu] — Autosuggest prefers own history (BA-103)

Next-word prediction weights the signed-in user’s own channel messages and session own-memory (other channels already loaded) above peers (`WEIGHT_OWN` 4 vs `WEIGHT_PEER` 1); the current composer draft trains at `WEIGHT_DRAFT` 3. Tests cover own-vs-peer ranking and draft ingest. Docs updated. PR #90.

## 2026-11-07 01:16:49 -06:00 [ubuntu] — Chat compose autosuggest (BA-103)

Bottom composer shows suggestion chips and ghost text while typing: `@` member mentions, optional `:emote:` shortcodes when names are supplied, and next-word / phrase completions from a local bigram/trigram model over recent channel history plus static common chat phrases. Tab or click accepts; Escape dismisses. Module `crates/banter/src/suggest.rs` with contract tests; UI wired in `draw_composer`; `SUGGEST_*` theme tokens. Docs: `docs/chat-autosuggest.md`. Branch `cursor/chat-autosuggest-0099`. Do not merge until Owl says so.

## 2026-11-07 01:25:20 -06:00 [ubuntu] — Discord-like Server Settings window (BA-106)

Dedicated Banter Server Settings OS window (Overview / Roles / Badges / Member List) opens from the server header gear, server dropdown, or rail right-click. Overview wires nickname + invite; Roles hosts live CRUD; Member List shows hoist preview and assign; Badges is Coming soon until community-badge APIs exist. Shares the secondary GLFW/ImGui host with User Settings (mutually exclusive). Unblocks BA-23 walk inside Settings. Do not merge until Owl says so.

## 2026-11-07 07:15:00 +00:00 [ubuntu] — SFU ForwardTable cleanup on leave (BA-99)

Pair-subscribe left stayer→departed publisher rows in `ForwardTable`, and `leave_voice` never called `unsubscribe_user`. Expand unsubscribe to drop publisher-side subs, publishing layers, and egress; wire leave and channel hops; generation-guard `accept_offer` teardown so mid-call re-offers cannot remove the replacement slot.

## 2026-11-07 07:02:16 +00:00 [ubuntu] — Fix call memory growth (BA-99)

Linux ffmpeg hardware encode kept every Annex-B access unit in the stdout capture `Vec` for the life of the encoder (copy from `before..` without drain), so RSS grew for the whole camera / Go Live session. Drain after each AU, clear on timeout/start, and hard-cap at 8 MiB (decoder path too). Cap H.264 receive AU reassembly at 4 MiB when markers are missing. Clear/prune remote ImGui video textures and delivered-layer notes when leaving voice or when peers leave the roster.

## 2026-11-07 06:57:32 +00:00 [ubuntu] — Fix Windows Go Live 0x80070057 (BA-100)

Voice Connected showed raw `The parameter is incorrect. (0x80070057)` while Go Live worked on Linux and failed on Windows. BA-92 already fixed WASAPI process-loopback GetMixFormat on main (#53); the remaining toast was Windows xcap DXGI `Monitor::video_recorder` (Desktop Duplication E_INVALIDARG, same as xcap#264) stored via unsanitized `screen: {err}`. Windows now falls back to paced GDI `capture_image` Still feed when the recorder fails; speaker loopback adds AUTOCONVERTPCM; screen failures use `BANTER_VC_PUBLISH_FAILED`. Linux Cast/X11 paths unchanged.

## 2026-11-07 07:05:53 +00:00 [ubuntu] — Cover-flow fade, rise, glass settings chip (BA-96)

Stream Select picker now soft-fades in/out and rises the cover-flow from the bottom. Stream settings move into a bottom-middle frosted glass chip (layered translucent fill + highlight + noise acrylic approximation). Full-bleed overlay keeps the arched cover-flow; chip expands for quality + stream audio. Rebased onto main.

## 2026-11-07 06:58:43 +00:00 [ubuntu] — Improve video encoder quality/CPU (BA-101)

Stacked on BA-97 (#85). Safe encoder wins without raising interactive delay: process-cached `hardware_probe`, Auto/Hardware opens GPU first and defers OpenH264 until needed, PictureDecoder defers soft decode while HW is up, OpenH264 screen/camera QP floors, simulcast Half=`full/3` / Quarter=`full/10` (was area share), Linux NVENC/VA-API honor CBR/VBR (+ NVENC `-zerolatency 1`). Keeps #85 bitrate RC / Low / 1-thread / 1 s GOP / screen AQ/BG off. Contracts in `crates/banter/tests/encoder_quality.rs`. Do not merge until Owl says so.

## 2026-11-07 02:55:00 -05:00 [ubuntu] — List all ffmpeg H.264 encoders; stop settings 1 FPS

Encoder chips now mirror `ffmpeg -encoders` H.264 backends Adam has (NVENC, AMF, QSV, VA-API, Vulkan, V4L2 mem2mem, libx264, libopenh264) plus built-in OpenH264 and Windows Media Foundation. The Voice & Video encoder row was re-running `ffmpeg -encoders` every frame via an uncached prefer path — cache that scan so opening settings no longer drops Banter to ~1 FPS.

## 2026-11-07 02:05:00 -05:00 [ubuntu] — Scan available encoders and let you pick one

Voice & Video / Stream options now probe which H.264 encoders this PC can see (OpenH264 always; NVENC, VA-API, Media Foundation when present) and only offer those chips, plus Auto and Any hardware. A saved backend that disappears stays visible but disabled with a fallback note, and open falls back to OpenH264 instead of assuming missing hardware exists.

## 2026-11-07 07:22:30 +00:00 [ubuntu] — Custom Mac-style title bar (BA-105)

Replaced native OS window chrome with an undecorated GLFW window and a short dark client-drawn caption: Mac traffic lights on the left (close / minimize / maximize), centered label, drag-to-move, and double-click maximize. Applies to Banter, Banter Settings, and Banter Stream; hidden in OS fullscreen. Tokens in `theme.rs`, geometry in `titlebar.rs` / `layout::Region::TitleBar`, paint in `skin`, GLFW actions in `platform`. Linux/Wayland keeps traffic lights and maximize; drag-to-move is a no-op when `glfwGetWindowPos` is unavailable.

## 2026-11-07 00:51:26 -06:00 [ubuntu] — Cut interactive VC latency (BA-97)

Full-stack latency pass (not encoder-only): playout deque ~60 ms (was ~80 ms after prior cut from 200 ms), capture/Opus PCM backlog ~200 ms (was 1–2 s), video RTP sample duration follows encode FPS, default keyframe 1 s, OpenH264 bitrate RC + Low complexity + 1 thread (screen disables AdaptiveQuant/BackgroundDetection), Opus complexity 5, encode tick floor 2 ms, SFU forward caps 24/16. Contract tests in `crates/banter/tests/vc_latency.rs`. Favor interactive delay over archive quality. Do not merge until Owl says so.

## 2026-11-07 06:00:00 -05:00 [ubuntu] — Version-1-test-13.2 pre-release

Shipped pre-release Version-1-test-13.2 from main after encode-stack optimizations (#83), hardware force-key IDR restore (#79), Windows MF hardware through force_key (#81), and 40 Mbps stream bitrate cap (#82). Older tags stay. Compose default remains `banter-server:local`. GHCR still serves `version-1-test-8` until a classic PAT with `write:packages` pushes this build.

## 2026-11-07 03:35:00 -05:00 [ubuntu] — Fix Windows Media Foundation streams (BAN-35)

BAN-30 kept hardware alive for SPS-less P-frames, but Windows MF still returned Err on `force_key` (LayerEncoder permanently swapped OpenH264 mid-GOP) and could keep non-Annex-B samples. Reopen the MFT on force_key/size change with a NEED_MORE_INPUT flush retry, and reject hardware AUs that are empty or not Annex-B while still allowing SPS-less Annex-B deltas.

## 2026-11-07 02:45:00 -05:00 [ubuntu] — Restore hardware force-key IDR after 13.1 (BAN-33)

Version-1-test-13.1 kept VA-API on GPU (#76) by skipping ffmpeg reopen on `force_key`, which left late-join PLI, P2P peer-cap keys, and mesh-flip recovery without an IDR on hardware encode. Reopen on `force_key` again (async_depth=1 still avoids the OpenH264 fallback), and do not burn `control.force_key` while no camera/screen layer is publishing.

## 2026-11-07 03:17:46 +00:00 [ubuntu] — Cover-flow brightness falloff (BA-96)

Matched the reference GIF more closely after video review: edge panels dim (scale + brightness), tighter horizontal pitch for overlap, no on-panel titles (hover tooltip for the source name instead).

## 2026-11-07 03:15:54 +00:00 [ubuntu] — Cover-flow stream switcher previews (BA-96)

Screen Share / stream source picker previews are now an arched horizontal cover-flow of tall portrait panels instead of a Discord-style landscape thumbnail grid. The focused source is centered and tallest; height falls off toward the left and right edges. Mouse wheel, left/right arrows, and click cycle focus; settling on a panel selects (or live-switches) that source. Dark translucent picker chrome; landscape captures cover-fit into portrait panels. Layout math lives in `coverflow.rs` with integration tests; picker metrics updated in `theme.rs` / `ui.rs`.

## 2026-11-06 04:00:00 -05:00 [ubuntu] — Version-1-test-13.1 pre-release

Shipped pre-release Version-1-test-13.1 from main after hardware encode stays on VA-API (#76 / BAN-30) and RTC restarts on guild P2P↔SFU flips (#77 / BAN-31). Older tags stay. Compose default remains `banter-server:local`. GHCR still serves `version-1-test-8` until a classic PAT with `write:packages` pushes this build.

## 2026-11-06 03:50:00 -05:00 [ubuntu] — Restart RTC when guild P2P↔SFU flips (BAN-31)

Go Live / camera stayed dead after guild opt-in P2P toggled or a third person joined because `Rtc::set_session` ignored topology when the offerer bit was unchanged (lower user id). Session key now includes `mesh`; P2P↔SFU flips rebuild the PeerConnection and clear stale WantedLayers / peer caps so multi-person guild SFU streams work again alongside DM mesh.

## 2026-11-06 02:50:00 -05:00 [ubuntu] — Version-1-test-13 pre-release

Shipped pre-release Version-1-test-13 from main after Banter UI sound events (join/leave/mute/stream + one-shot DM ringtone with call timeout), stream debug/encoder settings + late-join PLI + P2P streaming (#74), and Version-1-test-12 features. Older tags stay. Compose default remains `banter-server:local`. GHCR still serves `version-1-test-8` until a classic PAT with `write:packages` pushes this build.

- Release: https://github.com/Banter-Foundation/Banter/releases/tag/Version-1-test-13
- Assets: Banter-windows-version-1-test-13.zip, Banter-linux-version-1-test-13.zip, Banter-macos-arm-version-1-test-13.zip (Banter.app), banter-server.exe, banter-server-linux, banter-server-macos-arm

## 2026-11-06 02:10:00 -05:00 [ubuntu] — Fix DM P2P streaming + opt-in guild P2P video

DM Go Live/camera stayed invisible because call flag posts never reached the peer (`set_flags` only published voice-channel rosters) and P2P `SubscribeLayer` never forced an IDR. Active calls now rebroadcast `Call` with `caller_flags`/`callee_flags`; the stage seeds `voice_members` from those flags and still infers from remote textures. Publishers force a keyframe when a P2P peer cap changes, and idle camera/screen encoders clear when not publishing.

Guild voice stays on the SFU by default. Voice & Video gains **Prefer P2P video (1:1 voice channels)** (off by default): when both seated peers opt in and the channel has exactly two people, RTC meshes P2P and skips the SFU; otherwise SFU. Protocol bumped to 2 for `GuildP2pVideo` / roster `prefer_p2p_video`.

## 2026-11-06 01:55:00 -05:00 [ubuntu] — Late-join Go Live + SFU ports in /debugnow

Joining a voice channel while someone is already streaming now PLIs the publisher (accept_offer drained keyframe queue; BindPeer re-asks; client re-subscribes live screen/camera). `/debugnow` lists configured SFU UDP range and ports currently claimed. Publisher forces an IDR when WantedLayers subscriber count rises.

## 2026-11-06 01:45:00 -05:00 [ubuntu] — /debugnow stream info + OBS-like encoder settings

`/debugnow` shows a Stream section when local Go Live/camera is up or someone else is publishing: encoder backend, preference, rate control, keyframe interval, target vs measured kbps, encode size/FPS, and remote tile summary. Voice & Video / Stream options gain OBS-style encoder (auto/hardware/software), CBR/VBR, bitrate, and keyframe interval; those knobs feed the live H.264 encode path (still Constrained Baseline; OpenH264 fallback).

## 2026-11-06 00:08:11 -05:00 [ubuntu] — Version-1-test-12 pre-release

Shipped pre-release Version-1-test-12 from main after restore A/V after auto-reconnect (#72 / BAN-18), UI shell HTTP off the main thread, Purple Glow theme mod, and Version-1-test-11.3 features. Older tags stay. Compose default remains `banter-server:local`. GHCR still serves `version-1-test-8` until a classic PAT with `write:packages` pushes this build.

- Release: https://github.com/Banter-Foundation/Banter/releases/tag/Version-1-test-12
- Assets: Banter-windows-version-1-test-12.zip, Banter-linux-version-1-test-12.zip, Banter-macos-arm-version-1-test-12.zip (Banter.app), banter-server.exe, banter-server-linux, banter-server-macos-arm

## 2026-11-05 19:20:24 -0500 [dust] — UI thread stays off shell HTTP

Signed-in shell lists, channel/DM history, and avatar downloads run on one background worker. A frame only applies a snapshot whose generation is still current, so navigating and the 2-second refresh no longer freeze input behind a chain of GETs. One-shot writes (send, join, login) still run on the click that needs the response.

## 2026-11-05 16:30:00 -05:00 [ubuntu] — Purple Glow theme mod

Sample client mod `purple_glow` restyles the Banter shell to a dark purple neon look (rail, sidebar, stage, channel rows, composer, accents) via `banter.theme.shell` / `banter.theme.set`. Host applies more shell tokens for members, rows, muted/text, and composer. Copy from `examples/mods/purple_glow` or enable from a fresh data dir seed. Docs: Settings → Mods.

## 2026-11-05 19:16:00 +00:00 [ubuntu] — Version-1-test-11.3 pre-release

Shipped pre-release Version-1-test-11.3 from main after dismiss stream overlays on leave (#66), stream watch profile/voice indicators (#67), macOS Banter.app packaging (#68), and auto-reconnect every 30s with voice rejoin (#69). Older tags stay. Compose default remains `banter-server:local`. GHCR still serves `version-1-test-8` until a classic PAT with `write:packages` pushes this build.

- Release: https://github.com/Banter-Foundation/Banter/releases/tag/Version-1-test-11.3

## 2026-11-05 13:35:00 -05:00 [ubuntu] — macOS Banter.app without Terminal

Release zips had been shipping a bare `banter-macos-arm` Mach-O. Double-clicking that in Finder opens Terminal. Packaging now always wraps the client as `Banter.app` (`scripts/package-macos-app.sh` + `scripts/build-macos-release.sh`), with a CI layout check and a durable `macos-arm-release` workflow. `LSUIElement` stays unset (docked GUI app); the Terminal fix is the `.app` launch path, not an agent flag.

## 2026-11-05 18:19:11 +00:00 [ubuntu] — Dismiss stream overlays on leave

Leaving guild voice or a DM call (Disconnect / peer hang-up), or stopping a share, now clears Stream Select and Stream options. Previously those overlays could stay open until the next Go Live. Helper `dismiss_stream_overlays`; wired through `end_media_place` and `stop_streaming`.

## 2026-11-05 12:21:00 -06:00 [Winter] — Stream watch profile icons and voice indicators

Banter Stream pop-out and fullscreen theater chrome now show profile icons for live sharers with talking rings and mute/deafen badges. Those elements fade with the rest of the watch UI on pointer idle and return on mouse move. Avatar textures upload on the pop-out ImGui context like Settings.

## 2026-11-05 10:25:23 -06:00 [Winter] — Stream watch pop-out, cycle, fullscreen fade

Featured screen shares gain watch chrome: Pop Out opens a Banter Stream shared-GLFW window, Next Stream cycles live sharers, and Fullscreen fills the Banter window with chrome that fades after pointer idle and returns on mouse move (Escape exits). Logic in `stream_watch.rs`; metrics `STREAM_WATCH_*`. Left encoder quality / mid-stream refresh controls alone for the parallel worker.

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

## 2026-11-03 12:31:26 -06:00 [Winter] — Prefer 720p camera capture and unstack voice-bar errors

Camera open no longer asks nokhwa for `AbsoluteHighestResolution` (then JPEG-downscales). It prefers ~`1280x720`, or the nearest available mode that is at least that size when possible, with FPS near 30 and MJPEG favored on ties so Facecam and Elgato stay on the working decode path. The voice-connection panel shows one plain device error in the subtitle instead of drawing every error on the same bottom row.

- `pick_preferred_camera_mode` / `CameraModeChoice` in `media.rs`; `open_camera` lists modes and applies the pick
- `voice_bar_error_lines` / `voice_bar_error_ys` in `settings.rs`; connection panel uses the first plain line; overlapping bottom draws removed
- Tests written first: `camera_capture` (6) and `voice_bar_errors` (4). Also passed: camera_frames 4, device_errors 4, device_error_text 2, client 26, settings 18, theme 16, server foundation 13
- README and design notes describe the 720p preference and the single voice-bar error

## 2026-11-03 12:32:48 -06:00 [Winter] — Collaborator docs for the current Banter build

Added under `docs/`: overview (stack, data paths, bind address), build-and-run (Windows/Linux, tests, Podman/Docker `ghcr.io/snowedinowl/banter`), architecture (crates and modules, voice/camera/stream), feature-status (honest working vs deferred), contributing (worktrees, commits, `.cursor/changelog.md`), and current-build (live notes from Black-Birb without disturbing other agents). README now links those docs. Worktree `D:\Projects\Banter-docs` on `cursor/banter-build-docs-1fda`; primary `D:\Projects\Banter` left to Camera/voice-bar. Observed existing `banter` pid 34512, health ok on `127.0.0.1:3970`, PrintWindow capture of Cool Test Server / # general.

## 2026-11-03 14:42:32 -06:00 [Winter] — Developer Local vs Remote server

User Settings gains a Developer section so the desktop client can switch between the loopback server and a chosen remote host without rebuilding.

- Settings nav: User Settings (My Account, Voice & Video) plus Developer
- Local uses `http://127.0.0.1:3970` (`DEFAULT_ADDR`) and still starts the embedded server when nothing is listening
- Remote accepts `host`, `host:port`, or an `http(s)://` URL; invalid input keeps the previous base and shows an error
- Choice persists in `dev-server.json` under the data directory (`BANTER_DATA` / `%LOCALAPPDATA%\Banter`)
- Switching reconnects HTTP and WebSocket; a token that fails on the new host clears the local session so you can sign in again
- Docs: overview, build-and-run, architecture, feature-status, README; tests in `crates/banter/tests/dev_server.rs` written first (8 passed) alongside client 26 and settings 18

## 2026-11-03 14:45:00 -06:00 [Winter] — UI theme and layout branch scaffold

Opened dedicated branch `cursor/ui-theme-layout-cf5e` in worktree `D:\Projects\Banter-ui-theme` from `origin/main` so theme/layout work does not fight the dirty Dev Settings checkout at `D:\Projects\Banter`. Scaffold only: Discord-temp skin stays workable. Rebased onto main after Developer settings landed (`5d5915c`).

- New `crates/banter/src/layout.rs`: named `Region`s, `ShellColumns`, `shell_columns` (theme-token widths); `discord_shell_columns` kept as alias
- Moved shell column math out of `logic.rs`; `theme` / `skin` module docs point at the boundaries
- Docs: `docs/ui-theme-layout.md`; README, architecture, contributing, and design notes updated
- Tests first: `crates/banter/tests/layout.rs` (region names, theme-backed columns, rect helpers)
- Verified: `cargo test -p banter --test layout --test theme` — layout 5 passed, theme 16 passed (`CARGO_TARGET_DIR=D:\Projects\Banter-ui-theme\target`)

## 2026-11-03 15:18:54 -06:00 [Winter] — Consolidate under Project Banter

Moved the primary Banter checkout from `D:\Projects\Banter` to `D:\Projects\Project Banter` (exact path; space in the name) without rewriting git history. Linked worktrees were remapped under `D:\Projects\Project Banter\worktrees\` and repaired with `git worktree repair`. `/worktrees/` is gitignored in the primary tree. Docs and agent notes now point at the new root. Never touched `E:`. Owl's previous `banter-server` was stopped for the move and restarted from the new `target\debug` path.

## 2026-11-03 15:24:45 -06:00 [Winter] — Tighten root .gitignore

Expanded .gitignore so build output, local Banter data/secrets, IDE/OS junk, linked worktrees, and agent scratch stay out of git. Cargo.lock stays tracked (app workspace). No already-committed junk needed git rm --cached; Owl's on-disk files were not deleted.

- Ignore /target/, /target-*/ (e.g. CARGO_TARGET_DIR dirs like target-device-polish/), and *.pdb
- Ignore dev-server.json, banter.db / banter.db-*, .env.* (keep .env.example), *.key
- Ignore /worktrees/, agent scratch (.agent/, .claude/, .codex/, .cursor-tmp/, .cursor/agent-scratch/)
- Ignore IDE (.idea/, .vscode/, .vs/, *.suo, *.user, *.swp) and OS junk (Thumbs.db, .DS_Store, Desktop.ini)
- Keep ignoring auth/session/voice/stream secrets, .env, *.pem, secrets/, smoke logs



## 2026-11-03 15:34:56 -06:00 [Winter] — Clean up Banter git branches

Merged path/docs hygiene PRs #3 (Project Banter folder) and #4 (.gitignore) into main. Left UI theme/layout PR #2 open. Deleted local and remote branches already contained in main (settings-fixes, ui-polish, device-polish, banter-build-docs, dev-server-settings, webrtc-screen-share, plus the merged #3/#4 heads). Removed obsolete worktrees Banter-docs, Banter-settings, Banter-ui, and Banter-handcheck; kept Banter-ui-theme. Never touched E:.


## 2026-11-03 15:44:17 -06:00 [Winter] — Docker setup guide and refreshed server image files

Refreshed the root Dockerfile (adds ca-certificates in the runtime image), docker-compose.yml, and .dockerignore for the current banter-server (including WebRTC SFU). Added docs/docker-setup.md as the Podman/Docker walkthrough. Linked it from README, build-and-run, and contributing. Verified with Podman build and health check on host port 3972. Never touched E:.


## 2026-11-03 15:47:40 -06:00 [Winter] — Fix MF Send bounds; Version-1-test-2 client pre-release

Fixed Windows Media Foundation encoder/decoder compile errors after the WebRTC merge (Send for COM-owned MFTs, ProcessOutput sample borrow). Release-built the desktop client and server. Published GitHub pre-release Version-1-test-2 with Banter-windows-version-1-test-2.zip and banter-server.exe. Did not move Version-1-test-1. Never touched E:.


## 2026-11-03 18:54:03 -06:00 [Winter] — Fix voice channel SFU audio and Opus quality

Voice channels dropped Opus when the SFU required an OnOpen SSRC→RID map and when the forward table treated unpublished audio as Off. Early trickle ICE before the SFU slot existed was discarded. Open-mic capture skipped quiet frames before Opus, which chopped speech and broke DTX. Playback backlog was trimmed to ~200 ms and the RTC media queue was enlarged.

- SFU: `inbound_rtp_layer` forwards audio without the SSRC map; audio defaults to layer Full; pending ICE is buffered until `accept_offer`
- Client: `mic_opus_frame` feeds quiet open-mic frames to Opus; mute / PTT-up still stop the track
- Tests first: foundation voice admit path, SFU unit forward, client `mic_opus_frame`
- Docs: `docs/streaming.md`, feature-status. Never touched E:.


## 2026-11-03 19:16:58 -06:00 [Winter] — Shared integration branch test/owl-fixes

Created branch test/owl-fixes from origin/main (89f41b0) for sibling Owl-fix work. Documented merge protocol in docs/test-branch.md: feature branches rebase onto and merge into test/owl-fixes (not main); clear commits; never touch E:. Worktree at D:\Projects\Project Banter\worktrees\Banter-owl-fixes.

## 2026-11-03 19:23:32 -06:00 [Winter] — Discord-style Stream Select UI

Restyled the Screen Share / Go Live picker to match Discord’s Stream Select: dark overlay card, underline Screens / Applications tabs, larger thumbnail tiles with blurple selection border and check badge, Quality and Stream audio under the grid, Cancel plus green Go Live (and red Stop Streaming when live) in the footer. UI-only; share/select/go-live logic unchanged. Theme constants `SHARE_*` in theme.rs. Worktree Banter-stream-select on cursor/stream-select-discord-d812, merging into test/owl-fixes.

## 2026-11-03 19:24:45 -06:00 [Winter] — Fix screen share so remotes receive Go Live

Voice-channel SFU dropped every screen (and camera) packet because `bind_declared_ssrc` fires OnOpen with `rid=None` for Banter's fixed publish SSRCs, and forward rows were keyed by that empty string instead of `q`/`h`/`f`. P2P tiles could also key under sender `0` until signaling set the peer id.

- SFU: `inbound_rtp_layer_for_packet` prefers RID, then `SCREEN_SSRC_Q` / `CAMERA_SSRC_Q` offsets, then Full; forwards use the resolved layer RID
- Client: P2P remote screen/camera pictures follow the live peer id; SFU `fwd-` ids still embed the publisher
- Tests first: foundation screen admit path, SFU unit forward after pair subscribe, rtc receive parse/peer id
- Docs: `docs/streaming.md`, `docs/feature-status.md`. Worktree `Banter-screen-share`. Never touched E:.

## 2026-11-03 19:24:57 -06:00 [Winter] — Discord-style one-on-one call stage

Starting or accepting a DM call now replaces that conversation's chat stage with a Discord-like call view. Ringing shows a large peer avatar, status text, and Accept/Decline; an active call shows two participant tiles and the shared call control bar. Theme helpers and tests cover when the stage opens and the ringing layout. Stage disconnect ends a call as well as leaving voice. Never touched E:.
## 2026-11-03 19:25:59 -06:00 [Winter] — Replace camera JPEG spray with H.264 path clarity

Camera local preview no longer JPEG-encodes every frame. Capture still feeds I420 into the existing WebRTC H.264 encode path (OpenH264 or hardware). Docs and stream.hardware now match that stack instead of "software JPEG".

- `media.rs`: RGBA camera preview helpers; wire path unchanged (PictureSlot → OpenH264/H.264)
- `ui.rs`: mirrored local camera from RGBA; hardware flag from probe
- Tests first: `crates/banter/tests/video_codec.rs`
- Docs: streaming, architecture, feature-status, design, memory
- Never touched E:. Branch merges into test/owl-fixes only.

## 2026-11-03 19:26:23 -06:00 [Winter] — Auto-open DM when adding a friend

When a friendship becomes active (accept an incoming request, or send a username that completes a mutual pending request), the client opens that friend's DM. Pending-only sends stay on Add Friend. Helper `open_dm_peer_after_friend_add` decides the peer after the friends list refreshes. Never touched E:.

## 2026-11-03 19:28:01 -06:00 [Winter] — Fix camera so remote peers see it

Voice-channel camera RTP was dropped when OnOpen never mapped SSRC→RID (same class of bug as Opus/screen). Rebased onto the shared SFU RID/SSRC receive path from PR #8; adds camera admit coverage and P2P camera peer tagging without regressing screen forward.

- SFU: same `inbound_rtp_layer_for_packet` path as screen; camera foundation admit + SSRC unit tests
- Client: `remote_picture_sender` / `parse_fwd_track` cover camera as well as screen
- Docs: `docs/feature-status.md`. Never touched E:.

## 2026-11-03 19:28:05 -06:00 [Winter] — Discord-style right-click context menus

Owl test item: there were no right-click menus. Added Discord-like floating context menus for messages, members (and friends/DMs), and channels, wired to foundation actions only (reply/edit/delete/copy, mention/message/call/add friend, invite/copy channel). Kick and ban stay deferred. Pure menu builders live in logic.rs with tests written first.

## 2026-11-03 19:28:50 -06:00 [Winter] — User profiles (view/edit basics)

Floating profile cards for foundation identity. Avatar upload already existed; this adds view for others and edit of display name (plus Change Avatar) on your own card. About Me and custom status stay deferred.

- Server: `GET /v1/users/{id}` with optional `server_id` for nickname; visible to self, friends, and shared-server members
- Client: `profile.rs` card state; open from chat avatars/names, member list, friends, and the user-bar avatar; Edit Profile / Message / Add Friend
- Tests first: foundation `user_profile_visible_*` and `crates/banter/tests/profile.rs` (15 server + 5 profile + existing client suite green)
- Docs: features, feature-status, architecture, design. Worktree `D:\Projects\Project Banter\worktrees\Banter-profiles` on `cursor/user-profiles-8837`, merging into `test/owl-fixes` only. Never touched E:.

## 2026-11-03 19:34:33 -06:00 [Winter] — Fix My Account avatar button overflow

Change Avatar and Remove Avatar were positioned from Change alone, so Remove clipped past the profile card. Right-align both as one group with card padding, like Discord.

- Helper `account_avatar_actions_origin_x` + settings test first
- `draw_account_settings` uses the group width for cursor X
- Worktree `Banter-account-avatar` on `cursor/my-account-avatar-overflow-ec98` → merge into `test/owl-fixes` only. Never touched E:.

## 2026-11-03 19:48:29 -06:00 [Winter] — Merge test/owl-fixes into main and rebuild for Owl try

Fast-forwarded main from 89f41b0 to cf6d4e994d6098b3ab9fe712ce57696b737c6a7e (origin/test/owl-fixes tip, includes merged PRs #6 #8 #10 and owl-fixes feature merges). No conflicts. Pushed origin/main. Rebuilt debug banter + banter-server under D:\Projects\Project Banter\target\, restarted one banter-server on 127.0.0.1:3970 and one Owl client. Health ok. Left PR #2 (UI theme/layout) open. Never touched E:.
## 2026-11-03 19:50:11 -06:00 [Winter] — Point Docker defaults at version-1-test-3

Retarget the published server image docs and compose default from `version-1-test-1` to `version-1-test-3` for the owl-fixes main ship pre-release. Older `version-1-test-1` / `Version-1-test-2` tags stay. Never touched E:.

- `docker-compose.yml` default `BANTER_IMAGE` → `ghcr.io/snowedinowl/banter:version-1-test-3`
- `docs/docker-setup.md`, `docs/build-and-run.md`, `README.md` image/tag references updated

## 2026-11-03 20:03:10 -06:00 [Winter] — Version-1-test-3 pre-release (Win+Linux+GHCR) and cleanup

Shipped pre-release Version-1-test-3 from main `0d08c40` after owl-fixes merge (already on main; no double-merge). Built Windows and Linux clients + servers, pushed `ghcr.io/snowedinowl/banter:version-1-test-3`, retargeted compose/docs. Cleaned merged cursor/* + `test/owl-fixes` branches and obsolete Banter-* worktrees; kept `cursor/ui-theme-layout-cf5e` / Banter-ui-theme (PR #2). Never touched E:.

- Release: https://github.com/SnowedInOwl/Banter/releases/tag/Version-1-test-3
- Assets: Banter-windows-version-1-test-3.zip, Banter-linux-version-1-test-3.zip, banter-server.exe, banter-server-linux
- Local Owl try: release banter-server + banter on 127.0.0.1:3970

## 2026-11-03 20:33:55 -06:00 [Winter] — Clean up canceled BA-50 channel type templates

Removed canceled channel-type templates work from agent bc-31605c6f under D:\Projects\Project Banter. No commits had landed; branch tip matched main. Never touched Banter-ui-theme or E:.

- Deleted local branch `cursor/channel-type-templates-adb6` (was 3522fdd; not on origin)
- Switched primary checkout back to `main`
- Discarded uncommitted edits in `crates/banter-server/src/{error,lib,model,store}.rs`
- Deleted untracked `crates/banter-server/src/channel_types.rs`, `crates/banter-server/tests/channel_types.rs`, `crates/banter/tests/channel_types.rs`
- No channel-type worktree under `worktrees/`; `Banter-ui-theme` kept

## 2026-11-03 20:51:10 -06:00 [Winter] — Discord profile parity

Replace the basic Banter profile card with Discord-parity view + customization editor (nameplate, avatar decorations, display-name style, theme & banner, profile effect/frame, Board/Activity/Wishlist widgets). Banter-owned cosmetics catalog; no Discord assets. About Me / pronouns / custom status ship with profiles.

- Server: profile columns + widgets/connections tables, PATCH /v1/me/profile, banner upload, /v1/profile/cosmetics
- Client: Discord-style view card + full customization editor; contracts in profile_customization tests
- Docs: foundation, feature-status, architecture, design, features. Branch cursor/discord-profile-parity-5c8b. Never touched E:.


## 2026-11-03 21:36:10 -06:00 [Winter] — Version-1-test-4 pre-release (Win+Linux+GHCR)

Shipped pre-release Version-1-test-4 from main `b110a9f` (profile parity PR #12 + Postgres/hot-swap + Board exhaustiveness fix PR #14). Built Windows and Linux clients + servers, pushed `ghcr.io/snowedinowl/banter:version-1-test-4` (and `:latest`), retargeted docs. Cleaned release/fix branches and agent-scratch + target-linux-release; kept `cursor/ui-theme-layout-cf5e` / Banter-ui-theme (PR #2). Never touched E:.

- Release: https://github.com/SnowedInOwl/Banter/releases/tag/Version-1-test-4
- Assets: Banter-windows-version-1-test-4.zip, Banter-linux-version-1-test-4.zip, banter-server.exe, banter-server-linux
- Docs retarget: README, docs/build-and-run.md, docs/docker-setup.md published tag → version-1-test-4 (compose default stays `:latest`)

## 2026-11-03 21:56:19 -06:00 [Winter] — Client starts without forcing local server (BA-54)

Desktop startup no longer hard-fails when the loopback server is missing. Local is started only for a logged-in Local session or when the user selects Local; a saved Remote is used as-is; fresh Local with nothing listening opens the UI and asks for a remote host.

- logic: `StartupLocalPolicy`, `startup_local_policy`, `startup_should_prompt_remote`, `startup_api_base` (tests first in `dev_server.rs`)
- platform: never `?` on ensure_local at process start; reuse listening local or prompt
- ui: `prompt_remote_server` opens sign-in Server options toward Remote
- docs: overview, architecture, build-and-run; features list updated

## 2026-11-03 22:02:00 -06:00 [Winter] — Docs retarget Version-1-test-4.1

Retarget published Docker/pre-release defaults to `version-1-test-4.1` / `Version-1-test-4.1` after the BA-54 client startup change. Older tags stay. Compose default remains `:latest`.

## 2026-11-03 22:02:23 -06:00 [Winter] — Version-1-test-4.1 pre-release (BA-54)

Shipped pre-release Version-1-test-4.1 from main `c6163fc` (client startup without forcing local + docs retarget). Built Windows and Linux clients + servers, pushed `ghcr.io/snowedinowl/banter:version-1-test-4.1` (and `:latest`). Cleaned release branches and `target-linux-release`; kept `cursor/ui-theme-layout-cf5e` / Banter-ui-theme (PR #2). Never touched E:.

- Release: https://github.com/SnowedInOwl/Banter/releases/tag/Version-1-test-4.1
- Assets: Banter-windows-version-1-test-4.1.zip, Banter-linux-version-1-test-4.1.zip, banter-server.exe, banter-server-linux
- Behavior: local only when logged in on Local or user picks Local; remote config skips local; missing local asks for remote

## 2026-11-03 23:11:00 -06:00 [Winter] — Release deploy status (GHCR scopes)

Documented Arch server + release hot-swap progress. Workflow on `main` lowercases the GHCR owner and logs in with `GHCR_WRITE_TOKEN`. Deploy secrets are set. Releases through `Version-1-test-6` still fail at image push: the write PAT must be a classic token with `write:packages` (current error: token scopes). No GHCR `banter` package exists yet; SSH deploy is untested until push succeeds. Manual path: local `docker build -t banter-server:local` + deploy script (skip-pull for local tags).

- Docs: `docs/docker-setup.md` Status section + troubleshooting; `docs/feature-status.md` container row

## 2026-11-04 00:12:12 -06:00 [Winter] — Fix SFU forward SSRC rewrite (BA-55)

Server voice channels stayed silent and peers never got camera/screen because the SFU cloned publisher RTP into forward tracks unchanged. `RTCRtpSender::write_rtp` drops packets whose SSRC/PT/extensions are not on that sender leg (errors are logged inside the webrtc driver, so `write_rtp` still returned Ok). `rewrite_forward_rtp` now stamps the forward-track SSRC and payload type and clears inbound header extensions before forward.

- `room.rs`: `rewrite_forward_rtp`; Forward rows store outbound SSRC/PT; OnRtpPacket uses them
- Tests: room unit + foundation `sfu_forward_rtp_rewrites_publisher_ssrc_for_write_rtp` (plus existing voice_channel_* admit tests)
- Docs: streaming.md, feature-status.md, architecture.md
- Branch: cursor/server-vc-media-receive-03f6 — merged to main as PR #19
- Never touched E:

## 2026-11-04 00:12:16 -06:00 [Winter] — UI follows display refresh rate (BA-56)

Client UI present rate tracks the monitor under the window. With VSync on, GLFW `Sync(1)` is reapplied when the window moves to another display or the active mode Hz changes. Stream capture FPS options are unchanged.

- `logic`: `DisplayRefresh`, `pick_display_refresh_hz`, `ui_swap_needs_update` (tests in `settings.rs`)
- `platform`: each frame syncs swap interval to the current display; Wayland falls back to the primary monitor
- Docs: architecture, features, design, README Display note


## 2026-11-04 00:18:19 -06:00 [Winter] — Group stream audio by channel and app

Stream Select / stream options list mix sources under **Audio channels** (`mic:` and `loop:` system devices) and **Apps** (`app:` process loopback) instead of one flat checklist. Multi-select mix and per-source volume are unchanged. Helpers: `MixSourceGroup`, `mix_source_group`, `grouped_mix_sources`. Client test `stream_audio_sources_group_by_channel_and_app`.

## 2026-11-04 00:19:33 -06:00 [Winter] — Fix profile games widget editor (BA-60)

Games In Rotation / Games I Like no longer use a raw JSON text field that reset on every incomplete keystroke. The editor uses title + tag rows with add/remove, capped at Discord board limits (5 / 20). Payload helpers live in profile.rs with a unit test.

## 2026-11-04 00:23:01 -06:00 [Winter] — Reddit-style reply threading (BA-57)

Text channels and DMs render messages as Reddit comment trees. Nested reply-to-reply stays persisted via `reply_to`. Collapse/expand is local. Reply opens an inline composer under the parent; the bottom bar posts top-level only.

- Client: `comment_tree_rows`, `comment_descendant_count`, `toggle_comment_collapsed`; chat UI tree, indent, fold controls, inline reply draft
- Server: `idx_messages_reply_to`; foundation test for nested channel + DM threads
- Docs: architecture threading section, feature-status, features list
- Also fixed a broken client friend-DM integration test that called sync `Store::open` (now HTTP client like the other live tests)

## 2026-11-04 00:24:59 -06:00 [Winter] — Disable chat file uploads (BA-61)

Composer attach no longer opens a file picker or queues upload bytes. The attach control (and any leftover pending-file send path) opens the existing Banter modal shell with the exact message `Currently Unavailable`. Saving/downloading attachments already on a message still works. Server attachment API left unchanged so foundation tests stay intact. Client-only UX gate until Owl re-enables uploads.

- `Modal::UploadsUnavailable` + `pick_attachment` / `submit_composer` guards in `crates/banter/src/ui.rs`
- Docs: `.cursor/features.md`, `docs/feature-status.md`

## 2026-11-04 00:35:16 -06:00 [Winter] — Discord 1:1 profile parity push (BA-62)

Close High gaps from the BA-60 review: Discord-style editor slot chrome, richer Banter-owned cosmetic/badge paint, board game cards, full center preview, banner/avatar deferred to Save/Cancel, badge row, and Activity/Wishlist data model (empty until user-authored).

- Server: badges catalog + `badge_ids`, `profile_activity`, `profile_wishlist`; `GET /v1/profile/badges`; ProfilePatch fields
- Client: slot thumbnails, procedural cosmetics/badges/game cards, Board/Activity/Wishlist editors, media drafts on Save
- Tests: client profile_customization 12; server profile_customization 8 (Postgres :5433)

## 2026-11-04 00:39:16 -06:00 [Winter] — Postgres cutover punch list (BA-63)

Landed the required Postgres cutover audit punch-list items so long-lived volumes and local docs match the Postgres runtime.

- Versioned boot migrations: `schema.sql` then `migrate::apply` with `schema_migrations` + `001_profile_customization` (`ALTER TABLE … ADD COLUMN IF NOT EXISTS` / profile tables). Contract test upgrades a 46917ad-era `users` table.
- Docs: `docs/build-and-run.md` Local Postgres port (`BANTER_PG_PORT` + matching `BANTER_DATABASE_URL`); Windows CI noted as `--lib` only; `docs/current-build.md` data layout is Postgres URL + `BANTER_DATA` (no live `banter.db`).
- Shipped `.env.example` for database URL, PG port, data dir, bind/port.
- Verified on Compose Postgres `127.0.0.1:5433`: migrations, profile_customization 7/7, foundation 18/18. Do not merge until Owl says so.

## 2026-11-04 00:43:33 -06:00 [Winter] — Profile presence, scope switch, connections, art (BA-62)

Finish the four remaining BA-62 parity items on PR #21: custom status on live presence, Main vs server profile editor switch (nickname), clickable connections, and richer animated cosmetics / procedural box art.

- Server: PresenceSnapshot + custom_status on WS/HTTP presence; load status on connect; broadcast on profile patch
- Client: PresenceInfo map; friends subtitle; Main/Server combo; open http(s) connections; animated paint + box-art bevels
- Tests: client 13, server 9

## 2026-11-04 00:49:40 -06:00 [Winter] — Settings as separate polished window (BA-64)

User Settings opens in its own **Banter Settings** GLFW window (shared OpenGL context) instead of a full-screen overlay, so the main Banter shell stays usable. Closing the window chrome, Escape, or the in-UI X clears recording/mic-test state and returns focus to the main window.

- `platform.rs`: create/destroy shared-context settings host; dual ImGui pass (`draw_main` / `draw_settings_window`)
- `settings.rs`: OS-window desire, focus one-shot, nav list helpers, narrow My Account avatar action stacking
- Settings chrome: clearer nav section gaps, page subtitles on My Account / Voice / Developer
- Tests first in `crates/banter/tests/settings.rs` (28 passed including prior refresh-rate cases)

## 2026-11-04 01:07:30 -06:00 [Winter] — Stay signed in on login (BA-66)

Sign-in and register show a **Stay signed in** checkbox (default on). Checked writes `client-session.json` so relaunch restores the session; unchecked keeps the token in memory only and clears any prior saved file. Log Out still clears the saved session.

- `session.rs`: `clear_token`, `persist_login`
- Login card checkbox + `persist_login` after auth
- Docs and client tests for persist / no-persist / clear

## 2026-11-04 01:13:47 -06:00 [Winter] — Docs retarget Version-1-test-7

Retarget published Docker/pre-release defaults to `version-1-test-7` / `Version-1-test-7` after Stay signed in (BA-66) landed on main. Older tags stay. Compose default remains `:latest`.

## 2026-11-04 01:20:28 -06:00 [Winter] — Fix DrawListMut nested borrow panic (BA-69)

dear-imgui-rs panics when two `DrawListMut` wrappers wrap the same window draw list. Banter held a draw list across helpers that acquired another (comment fold tool, profile cosmetics, board stroke/text). Scoped acquires so wrappers drop before nested get_window_draw_list calls.

- `ui.rs`: comment thread line, tool, profile view/customize/preview/banner/board/editor thumbs
- `board.rs`: rect/sticky, image, file card, paint_poly fill before stroke helpers

## 2026-11-04 01:20:36 -06:00 [Winter] — Clear media unused-code warnings (BA-68)

Windows builds warned about Linux-only screen-cast timing and ffmpeg hardware encode paths that were still compiled in.

- `media.rs`: gate `cast_since` (Cast feed start timeout) behind `cfg(target_os = "linux")` so non-Linux no longer assigns a never-read timer
- `encode.rs`: keep NVENC/VAAPI `Backend` variants, `FfmpegEncoder`, and ffmpeg probe helpers behind `cfg(target_os = "linux")`; Windows still uses Media Foundation. Imports for those paths are Linux-only too
- Verified: `cargo build -p banter` clean; `RUSTFLAGS=-Dwarnings cargo test -p banter --lib encode::tests` — 3 passed

## 2026-11-04 01:21:17 -06:00 [Winter] — Version-1-test-7 pre-release (BA-66)

Shipped pre-release Version-1-test-7 from main `537e1e3` (Stay signed in PR #27 + docs retarget PR #28). Built Win+Linux clients/servers, pushed `ghcr.io/snowedinowl/banter:version-1-test-7` (+ `:latest`). Kept `cursor/ui-theme-layout-cf5e` / Banter-ui-theme. Never touched E:.

- Release: https://github.com/Banter-Foundation/Banter/releases/tag/Version-1-test-7
- Assets: Banter-windows-version-1-test-7.zip, Banter-linux-version-1-test-7.zip, banter-server.exe, banter-server-linux

## 2026-11-04 01:29:50 -06:00 [Winter] — Docs retarget Version-1-test-8

Retarget published Docker/pre-release defaults to `version-1-test-8` / `Version-1-test-8` after DrawListMut panic fix (BA-69 / PR #31) and media unused-code warnings (BA-68 / PR #29) landed on main. Older tags stay. Compose default remains `:latest`.

## 2026-11-04 01:35:30 -06:00 [Winter] — Version-1-test-8 pre-release (BA-70)

Shipped pre-release Version-1-test-8 from main `80ece87` (DrawListMut PR #31, media warnings PR #29, docs retarget PR #32; Stay signed in from test-7). Built Win+Linux clients/servers, pushed `ghcr.io/snowedinowl/banter:version-1-test-8` (+ `:latest`). Kept `cursor/ui-theme-layout-cf5e` / Banter-ui-theme. Never touched E:.

- Release: https://github.com/Banter-Foundation/Banter/releases/tag/Version-1-test-8
- Assets: Banter-windows-version-1-test-8.zip, Banter-linux-version-1-test-8.zip, banter-server.exe, banter-server-linux

## 2026-11-04 02:02:48 -06:00 [Winter] — Clarify Reddit replies UI (BA-72)

Visual-only clarity pass on Reddit-style reply threading already shipped in #22. Behavior unchanged (DFS tree, collapse, inline reply, top-level bottom bar).

- Stronger nesting: 32 px indent, per-depth thread guides, L-connector to child avatar
- Clearer fold: compact chevron control on the guide + clickable “▸ N more replies”
- Clearer Reply: always-visible Reply link; accented inline composer (header, stripe, Cancel/Reply) vs bottom top-level bar hint strip
- Spacing: root gap vs tighter nested gap; soft blurple wash on the active reply parent
- Docs: design, features, architecture, feature-status

## 2026-11-04 08:04:53 +00:00 [ubuntu] — Adopt BFMD for chat text formatting (BA-71)

Implement BFMD v1 for Banter chat: parser/state machine, persist BFMD in message body (plain text still valid), ImGui run rendering, composer B/I/U/S toolbar, and §12 conformance tests. Not CommonMark.

- `crates/banter/src/bfmd.rs` — parse, serialize, flag apply/toggle
- `crates/banter/src/bfmd_paint.rs` — wrapped ImGui draw/measure for runs
- `ui.rs` / `platform.rs` — message rows + format toolbar; italic/bold-italic fonts
- `docs/bfmd.md` — in-repo copy of the Sickle-adapted spec
- Tests: `crates/banter/tests/bfmd.rs` + unit tests in `bfmd.rs`

## 2026-11-04 08:06:41 +00:00 [ubuntu] — Major repair of broken profile UI (BA-73)

Profile view and customization editor were drawing without a dedicated ImGui overlay window and without clipped columns, so panels overflowed, editor columns collided, Save/Cancel sat on top of the board, cosmetics pickers spilled past the left column, and board widget kind radios on one line blew past the board width.

- `profile.rs` / `theme.rs`: `profile_view_layout` + `profile_editor_layout` contracts (disjoint identity/board; three editor columns + footer Save/Cancel/close)
- `ui.rs`: overlay windows for view + customize; clipped scroll children for identity/left/preview/board; cosmetics-first left column; stacked widget kind radios; auto-sized board widget cards; wrapped preview About Me; Cancel left / Save right footer
- Tests: client `profile_customization` 17 passed (layout contracts included).

## 2026-11-04 12:56:51 -0500 [ubuntu] — Version-1-test-9 pre-release (guild voice)

Guild voice showed the talking glow and passed no media. Camera and screen packets now carry the MID and RID header extensions, so those tracks are accepted in a guild channel and in a direct call. Compose publishes UDP `40000-40049` on `banter-a` and `40100-40149` on `banter-b`. `BANTER_ADVERTISE_IP` rewrites SFU host candidates (Compose default `127.0.0.1`).

Shipped pre-release Version-1-test-9. Older tags stay. Compose default remains `:latest`.

- Release: https://github.com/Banter-Foundation/Banter/releases/tag/Version-1-test-9
- Assets: Banter-windows-version-1-test-9.zip, Banter-linux-version-1-test-9.zip, banter-server.exe, banter-server-linux
- `room::tests::voice_channel_sfu_delivers_audio_camera_and_screen` passed (microphone, camera, and screen RTP).

## 2026-11-04 15:57:00 -0500 [ubuntu] — About Me, custom status, and guild roles

My Account saves About Me and a custom status. The status shows under your name and on member rows. Servers get an @everyone role plus extra roles with a color, an optional hoist, and permission bits. The server menu edits roles. A member's context menu assigns them. Kick and ban stay deferred.

- Migration `003_guild_roles`. Creating a channel or category requires Manage Channels. Creating an invite requires Create Invite. The owner has every permission.
- Tests: `roles::` 3 passed, `member_group` passed, `member_context_menu` passed, `guild_roles_gate_channels_and_stay_below_the_actor` and `server_invite_members_nickname_and_channels` passed against Postgres.

## 2026-11-04 16:03:09 -0500 [ubuntu] — Hidden /debugnow server debug window

Typing `/debugnow` in a text channel or DM opens a server debug window instead of sending a message. The window shows live stats and the server's recent log. It is not listed in any menu.

- `GET /v1/debug` (signed-in) returns uptime, RSS, presence, voice, calls, SFU slots, database counts, pool usage, and a 300-line ring
- Log lines also go to stderr: listen, login, logout, websocket, voice, calls, and server errors
- Tests: `is_debug_command`, debug log ring, live counts, and `debug_table_counts_include_new_rows` against Postgres

## 2026-11-04 16:12:33 -0500 [ubuntu] — Version-1-test-10 pre-release

Shipped pre-release Version-1-test-10 from main after About Me, custom status, guild roles, and the hidden `/debugnow` window. Older tags stay. Compose default remains `:latest`. GHCR still serves `version-1-test-8` until a classic PAT with `write:packages` pushes this build.

- Release: https://github.com/Banter-Foundation/Banter/releases/tag/Version-1-test-10
- Assets: Banter-windows-version-1-test-10.zip, Banter-linux-version-1-test-10.zip, Banter-macos-arm-version-1-test-10.zip, banter-server.exe, banter-server-linux, banter-server-macos-arm

## 2026-11-04 22:30:07 +00:00 [ubuntu] — Optional Lucide icons on channels (BA-78)

Channels can store an optional Lucide icon id. The sidebar and channel header show it when set, otherwise the kind defaults (hash / volume-2 / pencil). Create Channel and Edit Channel (context menu for Manage Channels) include a Lucide picker with a Default clear tile. Uploaded/custom image icons are not used (chat uploads stay disabled).

### Changelog
- New features: optional channel Lucide icons; Edit Channel modal; migration `004_channel_icons`
- Bug fixes: none
- Other: `PATCH /v1/servers/{id}/channels/{channel_id}`; create channel body accepts `icon`

- Migration `004_channel_icons` adds nullable `channels.icon` (also in tip `schema.sql`)
- Server allowlist `CHANNEL_ICON_CHOICES` / `normalize_channel_icon`
- Client picker, sidebar, and header use `channel_display_icon`
- Tests: foundation icon create/update/clear, migration column upgrade, client menu + display helper

## 2026-11-04 16:33:43 -0600 [ubuntu] — Nested server-rail folders with hex colors (BA-79)

The server rail can group servers into nested folders. Each folder has an optional `#RRGGBB` color. Collapse state, nesting, and order persist per signed-in user in Postgres.

- Migration `005_server_folders`: `server_folders` + `server_folder_memberships` (renumbered after `004_channel_icons`)
- API: `GET /v1/me/server-rail`, create/patch/delete folders, `PUT /v1/me/servers/{id}/folder`
- Client: rail tree with folder icons, context menus, create/rename/recolor modal (hex + RGB sliders)
- Nesting rules: per-user; folders under folders up to depth 8; delete promotes children; cycles rejected; invalid hex rejected
- Tests: foundation `server_folders_nest_hex_color_collapse_and_delete`, client `server_folders`, migrate `005_server_folders`

## 2026-11-04 23:19:41 +00:00 [ubuntu] — Home icon and customizable Home page (BA-81)

The top server-rail control (Discord-home spot) uses a Lucide home icon and opens a customizable Home landing instead of treating that button as a DM-only destination. Direct Messages remain in the sidebar and as a Home section.

### Changelog
- New features: Lucide `home` rail icon; Home landing with Friends / Direct Messages / Servers / Recent sections; Customize (show/hide + reorder); `home.json` persistence; Recent tracks opened DMs and servers
- Bug fixes: none
- Other: sidebar Friends still opens the full people list; Home tests in `crates/banter/tests/home.rs`

## 2026-11-05 00:11:39 +0000 [ubuntu] — BA-82 Reddit-style reply thread lines

- Match collapse chrome + vertical gutters to Reddit dark-mode reference (no L-hooks)
- Theme: 24 px indent, 1 px muted blue-gray guides, circular (−)/(+) fold, line-hit hover/click
- `CommentRow.ancestor_ids` maps each guide to the branch it collapses
- Drop DrawListMut before fold/line hit targets (dear-imgui #31 lesson)
- Tests: theme reddit_* helpers; comment_tree ancestor chain

## 2026-11-04 21:39:07 -05:00 [ubuntu] — Modding, plugin, and bot API docs

The extension API is written down and not implemented. The client and server still do not load mods, plugins, or bot accounts.

- [docs/modding.md](../docs/modding.md): sandbox, permissions and intents, immediate-mode render function, soft errors, hard errors, warnings, and export
- [docs/mods.md](../docs/mods.md), [docs/plugins.md](../docs/plugins.md), and [docs/bots.md](../docs/bots.md): the three references
- Linked from the README, architecture, overview, and contributing docs

## 2026-11-04 21:45:23 -05:00 [ubuntu] — Modding docs live under docs/modding

The four specification pages moved into `docs/modding/` so the modding references sit together. README, architecture, overview, and contributing point at the new paths.

## 2026-11-04 21:54:41 -05:00 [ubuntu] — Mod memory budget and per-mod process rate

A mod declares `memory_mb`. Asking for more than 150 MB adds a warning before it loads. Each mod keeps its own process rate, locked at 1,000,000 instructions and 50 ms, unless that user turns the lock off for that mod.

## 2026-11-04 21:58:22 -05:00 [ubuntu] — Mod theme and load order

Mod widgets start on the Banter theme. A mod can replace that default, restyle the Banter window, and pass a theme table on a single element. Enabled mods load, render, and receive events in one list the user can reorder.

## 2026-11-04 22:06:29 -05:00 [ubuntu] — Plugin client install choice

A plugin manifest sets `client` to `none`, `optional`, or `required`. `none` is server only: members are not prompted and there is no client bundle. `optional` and `required` ship a client script the member accepts before it runs.

## 2026-11-05 03:31:46 +0000 [ubuntu] — Fix Banter Settings dead UI and close (BA-84)

Settings opened and drew but ignored clicks/Escape/X and could not close cleanly. Dual ImGui NewFrame left `AppFocused` false after the main pass (GLFW Focus is transition-only), and glfw-rs `create_shared` windows skipped `glfwDestroyWindow` on Drop (zombie OS window).

### Changelog
- Bug fixes: per-pass ImGui focus from `receive_pointer`; Settings input when focused or hovered; poll mouse buttons on the Settings surface; destroy shared Settings GLFW window by clearing `is_shared` before Drop
- Other: platform tests for focus contract and shared-window destroy; settings close/Escape desire tests


## 2026-11-05 03:52:43 +0000 [ubuntu] — Banter Settings ImGui input still dead after #47 (BA-85)

After #47, Win11 OS chrome close worked but nav/toggles/in-UI X/Escape stayed dead. Shared Settings GLFW can queue CursorPos/MouseButton/Key while `is_focused`/`is_hovered` lie and main stays focused; BA-84 only routed on focus/hover, so ImGui never saw those events. Dual NewFrame also emitted Focus(false) on the main pass first, clearing key/mouse durations before Settings ran.

### Changelog
- Bug fixes: route Settings ImGui IO when its event queue has client input or the cursor is inside content; run Settings NewFrame first; do not emit Focus(false) on the sibling main pass; feed mouse buttons from the queue again; poll Escape
- Other: platform tests for queue/cursor routing contracts

## 2026-11-05 04:17:12 +0000 [ubuntu] — Banter Settings own ImGui context (BA-86)

After #47+#48 on main, Settings still drew with dead ImGui (OS chrome could close). Dual NewFrame on one ImGui context kept poisoning input despite focus/queue routing. Banter Settings now has its own Dear ImGui context (suspend/activate) and always feeds that window's GLFW input into it.

### Changelog
- Bug fixes: separate ImGui context + GlowRenderer for Banter Settings; own-window IO always; main suspends while Settings runs; Settings GPU maps/fonts swapped for that pass
- Other: platform tests for suspend/activate independence and main pointer suppress while Settings focused; architecture note

## 2026-11-05 04:44:56 +0000 [ubuntu] — Voice request failed notice + error catalog (BA-87 / BA-88)

The red stage toast showed opaque `request failed` while Voice Connected. That string was the client fallback for an empty HTTP error body on `POST /v1/voice/flags` (flags sync after join). Introduced a shared `BanterIssue` catalog and surface real codes/messages for voice join/leave/flags and WS connect/drop, with a 2s flags retry backoff.

### Changelog
- Bug fixes: never toast bare `request failed`; voice flags/join/leave wrap with `BANTER_VC_*`; parse server `code`; backoff flags republish
- New features: shared error catalog (`errcode.rs`, `docs/errors.md`); server `BanterError::code()` uses `BANTER_*` tokens
- Other: api_errors + errcode tests; architecture/README links

## 2026-11-05 05:05:04 +0000 [ubuntu] — Version system and version checker (BA-91)

Live hosts answering only `{"ok":true}` left newer clients talking to pre–PR #19 SFU builds with silent VC. Added a Banter release label plus numeric protocol on health/version, and a client checker that blocks stale/incompatible hosts with `BANTER_*` codes (protocol floors, not equal strings).

### Changelog
- New features: `version.rs` (`BANTER_VERSION`, `PROTOCOL`, min floors); `GET /v1/health` + `/v1/version` payload; client check on connect / auth / Developer reconnect; Developer settings version lines
- Bug fixes: refuse pre–version-system and too-old servers before silent feature mismatch
- Other: `docs/version.md`; server + client version contract tests; architecture/README links; version `BANTER_*` codes live in `errcode.rs` with the BA-88 catalog

## 2026-11-05 00:06:30 -0500 [ubuntu] — Compose builds the current server

`ghcr.io/snowedinowl/banter:latest` is still `version-1-test-8`, so Compose no longer uses that tag by default. `docker compose up --build` compiles `banter-server:local` from this checkout. The deploy script does the same when `BANTER_IMAGE` is unset, and if a registry pull fails it builds that local image instead.

- `docker build --pull -t banter-server:local .` succeeded (`d0530f12d92e`). The image contains `/usr/local/bin/banter-server`.
- A published tag still works when `BANTER_IMAGE` is set and the pull succeeds. The script then sets `BANTER_PULL_POLICY=never` so Compose does not rebuild over it.

## 2026-11-05 00:07:27 -05:00 [ubuntu] — Modding runtime

Mods, plugins, and bots now run from the modding spec. A missing permission returns nil, a missing bot intent returns JSON null, and a plugin can be server-only.

### Changelog
- Features: Luau host with permission nils, memory cap, and the per-mod process-rate lock; Settings → Mods load order, header widgets, and shell colors; plugin zip install with `client` none/optional/required; bot accounts, tokens, guild accept, and intent filtering
- Other: migration `006_modding`; foundation and migration tests still pass

## 2026-11-05 00:24:48 -05:00 [ubuntu] — Mods settings area

Settings has its own Mods section. The page lists the mods folder, the load order, and installed mods that are off, with enable, process-rate unlock, and move up or down.

## 2026-11-04 23:27:11 -06:00 [Winter] — Fix voice 0x80070057 WASAPI loopback (BA-92)

Guild Voice Connected showed a red `The parameter is incorrect. (0x80070057)` while Owl had screen share and Brave app-audio enabled. Process loopback called `GetMixFormat` on `VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK` (unsupported) and initialized with no `LOOPBACK` flag. Failures could surface as a raw Windows HRESULT under the green Connected label.

- Process loopback uses the Microsoft ApplicationLoopback PCM format (44.1 kHz stereo 16-bit) with `AUDCLNT_STREAMFLAGS_LOOPBACK | AUTOCONVERTPCM`
- `start_process` / `start_render` wait for open success/failure instead of swallowing pump errors
- Voice-bar notices use `BANTER_VC_LOOPBACK_FAILED` / `BANTER_VC_DEVICE_FAILED` (PR #50 catalog); raw HRESULT stays in detail/stderr
- Docs: `docs/errors.md`; tests: `errcode_loopback`, `wasapi_loopback_errors`

## 2026-11-05 00:35:00 -0500 [ubuntu] — Keep the mic off the screen encoder

Screen H.264 was encoded on the same thread as Opus, so a slow share held the next voice packet and the shared queue dropped it. Wayland also scaled every PipeWire buffer on the compositor thread.

- `banter-audio` encodes the microphone and stream audio. `banter-video` encodes camera and screen. Each has its own queue. A full video queue drops a picture, not Opus.
- The largest simulcast layer is encoded first.
- The PipeWire callback copies the newest frame. The screen thread scales it when that frame is published.

## 2026-11-05 00:38:37 -05:00 [ubuntu] — Sample mods and plugins

`examples/` has three client mods and three plugin packages that match the host APIs that already run.

## 2026-11-05 00:41:40 -05:00 [ubuntu] — Sample mods in the data directory

Settings → Mods reads `~/.local/share/Banter/mods`. The three sample mods are installed there, and a data directory with no mods folder receives them on startup. The Mods page rescans while it is open.

## 2026-11-05 05:46:54 +00:00 [ubuntu] — Remove text formatting toolbar (BA-94)

Owl asked to remove the BFMD B/I/U/S format toolbar from the composer and inline reply UI. Keep BFMD parse/render so pasted or typed style commands still work.

- `ui.rs`: drop `draw_format_toolbar`, `BfmdPending`, and related apply/flush helpers; reclaim the 28 px toolbar slot in the bottom composer and inline reply box
- Docs: `.cursor/features.md`, `docs/feature-status.md`
- BFMD modules (`bfmd.rs`, `bfmd_paint.rs`) and conformance tests unchanged

## 2026-11-05 00:50:00 -0500 [ubuntu] — Show who is in a voice channel

People in a server could not see that someone had joined a voice channel until they joined it too. The server only told the people already in that channel, and the client only drew that list under the channel it had joined. Opening a server also had no snapshot of who was already there.

- `ServerEvent::Voice` goes to every member of the server, including mute, camera, screen, and talking updates, and when someone leaves voice for a direct call.
- `GET /v1/servers/{id}/voice` returns every voice channel, including empty ones.
- The channel list and the unjoined voice stage use that roster.

## 2026-11-05 00:52:24 -05:00 [ubuntu] — Add Plugin from the local plugins folder

Right-click a server and choose Add Plugin. The list is the folders and zip files in the data directory's `plugins` folder, next to `mods`. Choosing one uploads that package to the server.

## 2026-11-05 01:03:49 -05:00 [ubuntu] — Plugin allow dialog and visible Welcome

Adding a plugin asks before it runs and says what it will do. Welcome 1.1.0 changes every new channel message on that server to test.

## 2026-11-05 01:08:42 -05:00 [ubuntu] — Remove a plugin from a server

Right-click a server and choose Plugins. Each plugin already on that server has Remove. Removing it unloads the script and deletes the install, so new messages are left alone.

## 2026-11-05 06:49:00 +0000 [ubuntu] — Version-1-test-11 pre-release

Shipped pre-release Version-1-test-11 from main `8e1efdc` (+ `BANTER_VERSION` bump `dcbba80`). Includes channel icons, server folders, Home landing, Settings ImGui fixes, voice error codes, version checker, Compose local image default, WASAPI loopback fix, visible voice roster, mic/screen encode split, BFMD toolbar removal, and mods/plugins from the data directory. Older tags stay. Compose default remains `banter-server:local`. GHCR still serves `version-1-test-8` until a classic PAT with `write:packages` pushes this build.

- Release: https://github.com/Banter-Foundation/Banter/releases/tag/Version-1-test-11
- Assets: Banter-windows-version-1-test-11.zip, Banter-linux-version-1-test-11.zip, Banter-macos-arm-version-1-test-11.zip, banter-server.exe, banter-server-linux, banter-server-macos-arm

## 2026-11-05 07:35:00 +0000 [ubuntu] — macOS voice and screen share TCC

Bare `banter-macos-arm` never appeared under Privacy & Security, so Microphone / Camera / Screen Recording stayed denied and voice plus Go Live looked broken.

- Embed `crates/banter/macos/Info.plist` into the binary (`build.rs` `__info_plist` section)
- Request Screen Recording via `CGRequestScreenCaptureAccess` before capture; clear System Settings errors
- `scripts/package-macos-app.sh` wraps the binary as `Banter.app` (bundle id `foundation.banter.desktop`) and ad-hoc signs it
- Docs: README, build-and-run, streaming, feature-status, overview

## 2026-11-05 02:47:26 -05:00 [ubuntu] — /debugnow shows server network traffic

The hidden `/debugnow` window now shows HTTP, WebSocket, and SFU media bytes in and out (totals and per-second rates) so publish/view issues are easier to spot live.

### Changelog
- New features: server traffic counters in `/debugnow` / `GET /v1/debug`
- Bug fixes: none
- Other: none

- `ServerTraffic` on `ServerDebug`; counters in `debuglog`, HTTP body middleware, WebSocket frames, and RTP forwards
- Window section "Network (server in / out)" with HTTP / WS / Media rates and totals

## 2026-11-05 02:52:35 -05:00 [ubuntu] — /debugnow shows server version

The hidden `/debugnow` Stats section now shows the live server `BANTER_VERSION` label and protocol revision from `GET /v1/debug`.

### Changelog
- New features: server version + protocol on `/debugnow` / `GET /v1/debug`
- Bug fixes: none
- Other: none

- `version` / `protocol` on `ServerDebug` (serde defaults for older hosts)
- Stats line above uptime/rss: `version …    protocol …`

## 2026-11-05 07:50:07 +00:00 [ubuntu] — Camera keyframes and Go Live restart

Camera publish always stored `key: false`, so remotes never got an IDR and camera looked broken on every OS. Screen restart could stick on a failed open, a stale portal session, or a congestion `ScreenStop` ladder. First camera/size frames are keyed again; End → Go Live clears sticky failure, settles capture, closes the portal session, and resets idle ScreenStop.

## 2026-11-05 03:12:00 -05:00 [ubuntu] — Version-1-test-11.1 pre-release

Shipped pre-release Version-1-test-11.1 from main after `/debugnow` network traffic + version/protocol, macOS Banter.app TCC packaging, camera keyframes, and Go Live restart cleanup. Older tags stay. Compose default remains `banter-server:local`. GHCR still serves `version-1-test-8` until a classic PAT with `write:packages` pushes this build.

- Release: https://github.com/Banter-Foundation/Banter/releases/tag/Version-1-test-11.1
- Assets: Banter-windows-version-1-test-11.1.zip, Banter-linux-version-1-test-11.1.zip, Banter-macos-arm-version-1-test-11.1.zip, banter-server.exe, banter-server-linux, banter-server-macos-arm

## 2026-11-05 16:22:00 +00:00 [ubuntu] — Mid-stream resolution and frame rate

The stage stream-options menu (chevron beside Screen) now shows the same resolution and frame-rate chips as Voice & Video → Screen share and Stream Select. Changing them while live still saves `stream.json` and applies through the existing `finish_voice_flags` → `set_stream_choice` path. Stage layout and call-bar chrome are unchanged.

## 2026-11-05 11:44:53 -05:00 [ubuntu] — Version-1-test-11.2 pre-release

Shipped pre-release Version-1-test-11.2 from main after mid-stream quality controls (#62), mute/deafen talking indicator fix (#63), and stream pop-out / cycle / fullscreen fade (#64). Older tags stay. Compose default remains `banter-server:local`. GHCR still serves `version-1-test-8` until a classic PAT with `write:packages` pushes this build.

- Release: https://github.com/Banter-Foundation/Banter/releases/tag/Version-1-test-11.2
- Assets: Banter-windows-version-1-test-11.2.zip, Banter-linux-version-1-test-11.2.zip, Banter-macos-arm-version-1-test-11.2.zip, banter-server.exe, banter-server-linux, banter-server-macos-arm

