# Streaming

Voice channels and direct calls use WebRTC. The websocket carries signaling only. Media is UDP, DTLS, and SRTP.

## Topology

- A server voice channel terminates on the embedded SFU. The SFU forwards RTP. It does not decode or transcode.
- A direct call is peer to peer. The server relays the offer, the answer, ICE, and `SubscribeLayer`.
- Group calls are not a product surface. The direct-call messages are the mesh shape a later group call would use.

Both sides offer the same four tracks: `user_audio`, `stream_audio`, `camera`, and `screen`. Simulcast RIDs are `q`, `h`, and `f` (quarter, half, full), in that order on the `a=simulcast` line. The H.264 profile is Constrained Baseline, `profile-level-id=42e01f`. Opus is 48 kHz mono with in-band FEC and DTX on the microphone.

Voice-channel SFU notes (reliability):

- Inbound **audio** RTP is forwarded without waiting on an OnOpen SSRC→RID map (that map often missed the mic SSRC and dropped every Opus packet in a channel while 1:1 P2P still worked).
- The forward table treats unpublished audio as layer `Full` so the first packet is not dropped before `note_layer`.
- Trickle ICE that arrives before the SFU slot exists is buffered and applied when the offer is accepted.
- Open-mic quiet frames still feed Opus (DTX); mute / deafen / PTT-up stop the track. Speaking glow still uses the RMS / PTT level.

ICE is host candidates, `BANTER_STUN` (default `stun:stun.l.google.com:19302`, empty disables it), and optional `BANTER_TURN` with `BANTER_TURN_USER` and `BANTER_TURN_PASSWORD`.

## Layers and the slider

The publisher encodes each wanted spatial layer once. The sender congestion ladder steps down in this order and never lowers user audio: camera half, stream-audio half, screen half, camera cut, stream-audio cut, screen stop. Three congested samples step down. Eight clear samples step up.

A viewer’s quality slider on a remote screen or camera snaps to full, half, quarter, or off. The choice is kept for that publisher and track until the app quits. It sends `SubscribeLayer`. The SFU forwards only that layer, so the viewer’s downlink follows the slider. Off stops video for that track and leaves audio alone. The slider cannot ask for a layer the sender has paused. The tile shows the layer that actually arrived when it is lower than the slider.

If nobody wants a layer, the publisher is told to pause it (dynacast). The first subscriber to a paused layer asks the publisher for a keyframe.

## Capture

Microphones and speakers stay on the operating system’s normal devices. Desktop and application audio are a separate Opus track, not mixed into the microphone. Windows uses WASAPI loopback. Linux uses the PulseAudio sink monitor (`parec`) and, for one application, the PipeWire node for that process (`pw-cat`). The same `loop:` and `app:` source ids are used on both.

Camera and screen frames are I420, then H.264. Local previews can still be JPEG. The resolution and frame-rate pickers are caps. A prewarm cutover reconfigures the encoder on the next keyframe.

Hardware encode is probed and used when the bitstream is Constrained Baseline:

- Linux: `h264_nvenc` when `libnvidia-encode` is present, otherwise `h264_vaapi` when `/dev/dri/renderD128` and ffmpeg’s VA-API encoder exist.
- Windows: a synchronous Media Foundation hardware H.264 MFT, asked for Constrained Baseline. Async-only MFTs are not driven.
- Otherwise OpenH264. A hardware encoder that emits a different profile is dropped for that layer.

Receive tries a hardware decoder when one is present: NVDEC or VA-API through ffmpeg on Linux, and a synchronous Media Foundation hardware decoder on Windows. The Linux decoder must return a picture for the first access unit within 800 ms and later frames within 120 ms. The Windows decoder must advertise an NV12 output type before the first sample. Either failure leaves that track on OpenH264.

OpenH264’s high-level settings do not expose temporal layers, so the capture interval is the frame-rate cap.

## Bots

A bot subscribes the same way a person does: join the voice channel, offer the four tracks, and send `SubscribeLayer` for the publisher and track it wants. There is no separate bot API.

## Release gate

Linux and Windows must exchange user audio, stream audio, camera, and screen in both directions, through a voice channel and through a direct call, including one hardware-encoded screen and one software-encoded screen. This tree is developed on Linux. Windows paths compile under `cfg(windows)` and are not executed here. `.github/workflows/ci.yml` runs the non-GPU tests on both operating systems.
