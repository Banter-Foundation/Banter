# Streaming

Voice channels and direct calls use WebRTC. The websocket carries signaling only. Media is UDP, DTLS, and SRTP.

## Topology

- A server voice channel terminates on the embedded SFU. The SFU forwards RTP. It does not decode or transcode.
- A direct call is peer to peer. The server relays the offer, the answer, ICE, and `SubscribeLayer`. Active-call `VoiceFlags` (mute, camera, Go Live) are rebroadcast on the `Call` event so both peers keep a live roster; P2P `SubscribeLayer` forces a publisher IDR when the peer cap changes.
- **Opt-in guild P2P video** (Voice & Video → Prefer P2P video; **off by default**): when exactly two people are in a voice channel and **both** enable it, that pair meshes WebRTC peer-to-peer instead of the SFU. A third joiner or either peer turning the setting off returns the channel to the SFU. Preference is signaled with `ClientSignal::GuildP2pVideo` and appears on `VoiceMember.prefer_p2p_video`. The client RTC session key includes a `mesh` bit so P2P↔SFU flips rebuild the PeerConnection even when the offerer is unchanged (lower user id staying offerer; multi-person guild returning to SFU).
- Group calls are not a product surface. The direct-call messages are the mesh shape a later group call would use.

Protocol: this work bumps `PROTOCOL` to `2` (call flags + guild P2P signaling). Older clients that omit the new serde-default fields still connect (`MIN_CLIENT_PROTOCOL` remains `1`).

Both sides offer the same four tracks: `user_audio`, `stream_audio`, `camera`, and `screen`. Simulcast RIDs are `q`, `h`, and `f` (quarter, half, full), in that order on the `a=simulcast` line. The H.264 profile is Constrained Baseline, `profile-level-id=42e01f`. Opus is 48 kHz mono with in-band FEC and DTX on the microphone.

Voice-channel SFU notes (reliability):

- Inbound **audio** RTP is forwarded without waiting on an OnOpen SSRC→RID map (that map often missed the mic SSRC and dropped every Opus packet in a channel while 1:1 P2P still worked).
- Inbound **screen/camera** RTP prefers OnOpen RID, then Banter’s fixed publish SSRC bases (`SCREEN_SSRC_Q` / `CAMERA_SSRC_Q` + 0/1/2 for `q`/`h`/`f`), then `Full`. Waiting only on OnOpen dropped every Go Live / camera packet for remotes (`bind_declared_ssrc` leaves `rid=None`).
- Forward rows are keyed by the resolved layer RID (`q`/`h`/`f`), not by an empty OnOpen string.
- The forward table treats unpublished audio as layer `Full` so the first packet is not dropped before `note_layer`. Pair subscribe marks screen Full publishing so the first keyframe is admitted.
- Before `write_rtp` on a forward track, the SFU rewrites the packet to the track’s negotiated SSRC and payload type and strips inbound header extensions. Publishing SSRCs are not the forward-track SSRCs; leaving them unchanged made `RTCRtpSender::write_rtp` reject every Opus / H.264 packet (silence + blank remote tiles on server VC while 1:1 P2P still worked).
- Camera and screen are simulcast. The stack drops `a=ssrc` on a rid m-line and accepts a video SSRC only when the packet carries the negotiated MID and RID header extensions. Microphone audio is one encoding, so it does not need them. Direct calls use the same video packets.
- Trickle ICE that arrives before the SFU slot exists is buffered and applied when the offer is accepted.
- Open-mic quiet frames still feed Opus (DTX); mute / deafen / PTT-up stop the track. Speaking glow still uses the RMS / PTT level.
- P2P remote screen/camera tiles use the live peer id from signaling once it is known; SFU forward track ids already embed the publisher.

ICE is host candidates, `BANTER_STUN` (default `stun:stun.l.google.com:19302`, empty disables it), and optional `BANTER_TURN` with `BANTER_TURN_USER` and `BANTER_TURN_PASSWORD`.

Compose does not publish the SFU’s random UDP port, so a desktop can finish websocket signaling (the talking glow) and a direct call, and still receive no guild media. `BANTER_UDP_MIN` / `BANTER_UDP_MAX` bind each voice PeerConnection to that range. `BANTER_ADVERTISE_IP` rewrites host candidates to an address the client can reach (the compose default is `127.0.0.1`). Slot A publishes UDP `40000-40049`. Slot B publishes `40100-40149`. A client on another machine needs the host’s reachable IP in `BANTER_ADVERTISE_IP` and that range open. Direct calls stay peer-to-peer and do not use these ports.

## Layers and the slider

The publisher encodes each wanted spatial layer once. The sender congestion ladder steps down in this order and never lowers user audio: camera half, stream-audio half, screen half, camera cut, stream-audio cut, screen stop. Three congested samples step down. Eight clear samples step up.

A viewer’s quality slider on a remote screen or camera snaps to full, half, quarter, or off. The choice is kept for that publisher and track until the app quits. It sends `SubscribeLayer`. The SFU forwards only that layer, so the viewer’s downlink follows the slider. Off stops video for that track and leaves audio alone. The slider cannot ask for a layer the sender has paused. The tile shows the layer that actually arrived when it is lower than the slider.

If nobody wants a layer, the publisher is told to pause it (dynacast). The first subscriber to a paused layer asks the publisher for a keyframe.

P2P (DM DirectCall, and guild voice when both peers opted into P2P video) never receives SFU `WantedLayers`. A new or changed peer `SubscribeLayer` cap therefore forces a local IDR so Go Live / camera after the call is up (or a late watcher) can decode. Encode loops also skip camera/screen layers while the matching `publishing_*` flag is off, so a restart after toggle starts on a fresh keyframe instead of orphaned P-frames. Linux VA-API/NVENC has no mid-stream force-IDR on the pipe path, so a `force_key` reopens ffmpeg (safe with `-async_depth 1`); the encode loop does not consume that flag while no layer is publishing, so a mesh flip or WantedLayers arm before Go Live still yields an IDR on the first real frame.

## Capture

Microphones and speakers stay on the operating system’s normal devices. Desktop and application audio are a separate Opus track, not mixed into the microphone. Windows uses WASAPI loopback. Linux uses the PulseAudio sink monitor (`parec`) and, for one application, the PipeWire node for that process (`pw-cat`). The same `loop:` and `app:` source ids are used on both. macOS has microphone and speakers through Core Audio; speaker loopback and per-app stream audio are not wired yet.

On macOS, voice and screen share need Privacy & Security grants. Ship and run **`Banter.app`** from `scripts/package-macos-app.sh` / `scripts/build-macos-release.sh` so TCC can attribute Microphone, Camera, and Screen Recording to bundle id `foundation.banter.desktop`, and so Finder does not open Terminal (that happens when a bare Mach-O is double-clicked). A bare ad-hoc binary often never appears in those lists, so capture fails with no useful prompt. Banter calls `CGRequestScreenCaptureAccess` before the first Go Live.

In Stream Select / stream options, checkable mix sources are grouped: **Audio channels** (`mic:` and `loop:` system devices) and **Apps** (`app:` process loopback). Several can stay on and mix into one stream-audio track; each keeps its own volume.

Camera and screen frames are I420, then H.264. Local previews are RGBA (mirrored for the local camera). Stream-select thumbnails may still be JPEG stills. The resolution and frame-rate pickers are caps. They live in User Settings → Voice & Video → Screen share, in Stream Select, and in the stage stream-options menu (chevron beside Screen) so a live share can change quality without opening Settings. A prewarm cutover reconfigures the encoder on the next keyframe. Leaving a voice channel or call (Disconnect, peer hang-up), or stopping the share, dismisses Stream Select and Stream options so those overlays cannot linger after the place ends.

Viewers can **pop out** the focused share into a Banter Stream OS window, **cycle** among users who are currently sharing, and enter **fullscreen** theater where watch chrome fades after pointer idle and returns on mouse movement. Pop-out and fullscreen chrome include profile icons and voice indicators (talking ring, mute/deafen) for live sharers; those fade with the rest of the UI. Those controls do not change encoder resolution or capture FPS.

Hardware encode is probed and used when the bitstream is Constrained Baseline:

- Linux: `h264_nvenc` when `libnvidia-encode` is present, otherwise `h264_vaapi` when `/dev/dri/renderD128` and ffmpeg’s VA-API encoder exist.
- Windows: a synchronous Media Foundation hardware H.264 MFT, asked for Constrained Baseline. Async-only MFTs are not driven.
- Otherwise OpenH264. A hardware encoder that emits a different profile is dropped for that layer.

Receive tries a hardware decoder when one is present: NVDEC or VA-API through ffmpeg on Linux, and a synchronous Media Foundation hardware decoder on Windows. The Linux decoder must return a picture for the first access unit within 800 ms and later frames within 120 ms. The Windows decoder must advertise an NV12 output type before the first sample. Either failure leaves that track on OpenH264.

OpenH264’s high-level settings do not expose temporal layers, so the capture interval is the frame-rate cap.

Microphone Opus runs on `banter-audio` with its own queue. Screen and camera H.264 run on `banter-video`. A slow screen frame does not hold the next voice packet, and a full video queue drops a picture instead of an Opus packet. The largest simulcast layer is encoded first.

On Wayland the PipeWire callback only copies the newest desktop buffer. The screen thread scales that copy when it is time to publish, so the compositor is not blocked on a pixel walk of frames that will be dropped.

## Bots

A bot subscribes the same way a person does: join the voice channel, offer the four tracks, and send `SubscribeLayer` for the publisher and track it wants. There is no separate bot API.

## Release gate

Linux and Windows must exchange user audio, stream audio, camera, and screen in both directions, through a voice channel and through a direct call, including one hardware-encoded screen and one software-encoded screen. This tree is developed on Linux. Windows paths compile under `cfg(windows)` and are not executed here. `.github/workflows/ci.yml` runs the non-GPU tests on both operating systems.
