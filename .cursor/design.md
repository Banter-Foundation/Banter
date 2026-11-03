# Window

Dear ImGui docking branch, dark style. Fonts: Segoe UI on Windows when `C:\Windows\Fonts\segoeui.ttf` is present; on Linux Liberation Sans, then Noto Sans, then DejaVu Sans.

After login the window is a temporary copy of Discord's screen so the controls are familiar. It is not the final Banter look, and it does not use Discord's logo.

The signed-in window is a fixed column shell. Panels are not dragged.

- Far left, 72px: server rail. Home is the letter B, or the mention count. Each server is a round icon. The green plus opens create or join.
- Next, 240px: channel sidebar. Friends and direct messages, or the server name with a chevron menu for invite, nickname, and channel creation. Text rows use a hash icon. Voice rows use a speaker icon. People in the joined voice channel sit under that row. The bottom of this column is the voice connection, then the user bar.
- Center: the stage. Friends has Online, All, Pending, and Add Friend. A text channel has the message list. A voice channel is a dark stage: a large shared screen when someone is streaming, then tiles with camera or avatar, name, and mute or deafen icons.
- Right, 240px: members. Hidden on Home. The people icon in the header hides or shows it.

Icons are Lucide, drawn in white, with red for mute, deafen, and disconnect.

A text channel lists messages without a line under each one. The first message in a group has a 40px round avatar, a bold display name, then a gap and a smaller muted time ("Today at 10:17 AM"). Hovering the time shows the full date. A follow-up from the same person within 7 minutes on the same local day repeats neither the avatar nor the name, and shows the short time in the gutter while the row is hovered. A new local day gets one separator line. The row background changes on hover. A floating toolbar at the top right appears only then: reply, edit for your own message, delete, and more. More copies the message text. React is not shown.

The gear opens User Settings. Escape or the X closes it. The user-bar avatar also opens My Account. My Account edits the display name and avatar and shows the username. Change avatar picks a PNG, JPG, GIF, or WebP, then a circle crop with zoom and drag. Save stores it on the server. Remove avatar falls back to the letter. The same picture is used in chat rows, the member list, voice tiles, the user bar, and direct messages. Voice & Video has input and output devices with volume, a Let's Check meter, Voice Activity or Push to Talk, automatic or manual sensitivity, a push-to-talk key and release delay, and noise suppression, echo cancellation, and automatic gain control. Those choices are saved in the Banter data directory and restored on the next launch. Push to Talk defaults to V. A Talk button appears in the voice bar. The green ring follows the mic level in voice activity, and the held key in push to talk.

The video button turns the camera on or off. Capture prefers ~720p (`1280x720`) or the nearest available mode, with MJPEG favored when tied, so Facecam and Elgato stay on the working decode path. Device errors in the voice-connection panel replace the place subtitle with one plain message; they must not draw on the same row. User Settings still lists every device error. The screen button opens Screen Share, or stops a stream that is already live. A chevron beside it opens upward with encoder, bitrate, cursor, and preview. Screens and Applications are separate. A thumbnail is a real capture of that screen or window. Nothing is sent until Go Live. Resolution and frame rate are separate: 480p, 720p, 1080p, 1440p, 4K, or Source, and 15, 30, 60, 120, or 144 FPS. A choice the selected screen cannot do stays on screen with the reason. The default is 720p at 30 FPS. Hardware H.264 is not available on this PC, so the encoder stays software JPEG and the menu says so. Stream audio is a checklist of microphones, speaker loopback, and apps, each with its own volume. Changing source or quality while live keeps the same stream and cuts over when the next picture is ready. Disconnect, ending a call, and log out also stop the camera and the stream.

Clicking a voice channel joins it. Text chat inside a voice channel stays deferred.

The sign-in window is not docked. The password field is hidden as `*` until Show password is checked. Ctrl+C and Ctrl+V use the system clipboard in every text field, including a hidden password. The Ctrl, Shift, Alt, and Super modifiers are reported to Dear ImGui separately from the physical keys.

## Discord skin metrics (ui-polish)

All colors, sizes, radii, and font sizes come from `crates/banter/src/theme.rs`. Do not hard-code a color or pixel size in `ui.rs`; add a constant there instead. `crates/banter/src/skin.rs` holds the shared drawing helpers (text, buttons, inputs, menus, tooltips, shadow, checkbox).

- Columns: rail 72, sidebar 240, members 240. Header 48 with a 1 px shadow line. User panel 52, voice panel 96
- Rail: 48 px icons, 8 px gap, 12 px top. Pill 4 px wide, 8 px on hover, 40 px selected. Radius 24 (circle) idle, 16 hover/selected
- Rows: channel 32 px with 2 px gap and 8 px inset, radius 4. Category 40. Member 42. Avatar 32
- Composer 44 px, radius 8, 16 px side margin, 24 px bottom
- Call: buttons 56, gap 16, bar 96. Tiles 16:9, gap 8, radius 8, avatar 80
- Floating: tooltip pad 12x8 with a 5 px arrow, radius 4. Menu 220 wide, 32 px items. Modal 440 wide, radius 8, footer 70. Buttons 38 (44 on sign-in, 16 px label). Inputs 40, radius 3
- Fonts: 12/14/16/20/24 body sizes. Windows uses Segoe UI / `seguisb.ttf`. Linux uses Liberation Sans (then Noto / DejaVu), with Noto Sans Medium for semibold when present
- Status dot: 10 px on a 32 px avatar with a 3 px cut-out ring; idle is a crescent, offline a ring
- Screenshots of this pass: `ui-*.png` in the coordinating agent's media folder
