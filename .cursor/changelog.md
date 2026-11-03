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

## 2026-11-03 11:32:50 -06:00 [Winter] - User Settings fixes

Branch `settings-fixes`, worktree `D:\Projects\Banter-settings`. User Settings was rebuilt to Discord's layout and every control was checked by hand in a running build, including persistence after a restart.

- Layout: the settings overlay is opaque, the nav list and a 740 px content column center on wide windows, narrow windows shrink without overlap, long pages scroll, and the X and ESC button stays put while scrolling (`settings_layout`)
- Escape closes an open dropdown first, then cancels keybind recording, then closes settings. Dear ImGui only closes dropdowns on Escape with keyboard navigation on, and Banter leaves it off
- My Account: profile card, display name with the server's 1 to 32 character rule, Save only when changed and valid, Enter saves, and the user bar shows the saved name rather than the draft
- Voice & Video: device dropdowns, volumes as 0 to 200 percent, the mic test stops when settings close or the page changes, sensitivity in dB with the meter mark under the slider knob, push to talk keys A to Z, 0 to 9, and F1 to F12, release delay, and processing toggles saved on change
- Sliders are a track with a knob and the value printed to the right. The old Dear ImGui grab covered the centered value, so 100 percent output volume was always hidden
- Camera: the choice is saved by name, so it survives device order changes. MJPEG frames decode through `image`, because nokhwa's MJPEG path needs its `mjpeg` feature and otherwise says `Not available on WASM`. This is why the Elgato Facecam MK.2 never showed a picture. Switching cameras clears the previous picture, device errors are plain sentences that clear when the device works again, and the raw error goes to stderr once instead of every frame
- `voice.json` and `stream.json` from an older build still load, because missing fields take defaults
- Tests written before the code: `tests/settings.rs` 18, `tests/device_errors.rs` 4, `tests/device_error_text.rs` 2, `tests/slider.rs` 4, `tests/camera_frames.rs` 4. With `tests/client.rs` 24, 56 passed
- Not tested by hand: push to talk transmitting inside a voice call, the stream options popup from the voice bar chevron, and Log Out
