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
