# Window

Dear ImGui docking branch, dark style, Segoe UI when `C:\Windows\Fonts\segoeui.ttf` is present.

After login the window is a temporary copy of Discord's screen so the controls are familiar. It is not the final Banter look, and it does not use Discord's logo.

The signed-in window is a fixed column shell. Panels are not dragged.

- Far left, 72px: server rail. Home is the letter B, or the mention count. Each server is a round icon. The green plus opens create or join.
- Next, 240px: channel sidebar. Friends and direct messages, or the server name with a chevron menu for invite, nickname, and channel creation. Text rows use a hash icon. Voice rows use a speaker icon. People in the joined voice channel sit under that row. The bottom of this column is the voice connection, then the user bar.
- Center: the stage. Friends has Online, All, Pending, and Add Friend. A text channel has the message list. A voice channel is a dark stage: a large shared screen when someone is streaming, then tiles with camera or avatar, name, and mute or deafen icons.
- Right, 240px: members. Hidden on Home. The people icon in the header hides or shows it.

Icons are Lucide, drawn in white, with red for mute, deafen, and disconnect.

The gear opens User Settings. Escape or the X closes it. My Account edits the display name and avatar and shows the username. Voice & Video has input and output devices with volume, a Let's Check meter, Voice Activity or Push to Talk, automatic or manual sensitivity, a push-to-talk key and release delay, and noise suppression, echo cancellation, and automatic gain control. Those choices are saved in the Banter data directory and restored on the next launch. Push to Talk defaults to V. A Talk button appears in the voice bar. The green ring follows the mic level in voice activity, and the held key in push to talk.

The video button turns the camera on or off. The screen button opens Screen Share, or stops a stream that is already live. A chevron beside it opens upward with encoder, bitrate, cursor, and preview. Screens and Applications are separate. A thumbnail is a real capture of that screen or window. Nothing is sent until Go Live. Resolution and frame rate are separate: 480p, 720p, 1080p, 1440p, 4K, or Source, and 15, 30, 60, 120, or 144 FPS. A choice the selected screen cannot do stays on screen with the reason. The default is 720p at 30 FPS. Hardware H.264 is not available on this PC, so the encoder stays software JPEG and the menu says so. Stream audio is a checklist of microphones, speaker loopback, and apps, each with its own volume. Changing source or quality while live keeps the same stream and cuts over when the next picture is ready. Disconnect, ending a call, and log out also stop the camera and the stream.

Clicking a voice channel joins it. Text chat inside a voice channel stays deferred.

The sign-in window is not docked. The password field is hidden as `*` until Show password is checked. Ctrl+C and Ctrl+V use the system clipboard in every text field, including a hidden password. The Ctrl, Shift, Alt, and Super modifiers are reported to Dear ImGui separately from the physical keys.
