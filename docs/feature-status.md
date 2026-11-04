# Feature status

Honest snapshot of the Discord-like **foundation** vs what still waits. The authoritative deferred list is also in [foundation.md](foundation.md). Do not implement deferred items until Owl asks.

## Works in the current build

| Area | Status |
| --- | --- |
| Register / login / logout (local username + password) | Working |
| Display name + avatar (upload crop/zoom PNG/JPG/GIF/WebP, remove → letter) | Working |
| User profiles (Discord-parity popout + customization editor: banner, theme, cosmetics, bio, board widgets) | Working |
| Friends list + friend requests | Working — accepting (or completing a mutual request) opens that friend's DM |
| Create server, invite join, member list, server nickname | Working |
| Text + voice channels in categories (owner creates) | Working |
| Drawing board channels (shapes, stickies, images, files, realtime ops) | Working |
| Send / reply / edit / delete / attach / search messages | Working |
| Mentions + presence | Working |
| Discord-like temporary skin (rail, sidebar, chat, members, modals) | Working — **not** final Banter branding |
| Chat rows (avatar, grouping, hover toolbar, bottom-anchored history) | Working |
| Right-click context menus (messages, members/friends/DMs, channels) | Working — foundation actions only; no kick/ban/profiles |
| Drop-in voice: mute, deafen, PTT, connection panel | Working (SFU rewrite outbound SSRC/PT on forward; open-mic Opus timeline) |
| Camera on/off with mirrored local RGBA preview (~720p prefer; device MJPEG OK) | Working (device-dependent); wire is WebRTC H.264; remotes receive after RID map + forward SSRC rewrite |
| Screen share Go Live + quality / source switching | Working (WebRTC H.264 SFU/P2P; remotes receive after RID map + forward SSRC rewrite) |
| 1:1 DMs + start a call | Working — phone icon opens a Discord-style call stage (ringing avatar / two tiles when active) |
| User Settings: My Account, Voice & Video, Developer (Local/Remote API, persisted) | Working |
| Embedded + standalone server, PostgreSQL data | Working |
| Containerized `banter-server` (`ghcr.io/snowedinowl/banter`) | Image/Dockerfile ready; **GHCR publish + release auto-deploy blocked** until classic PAT has `write:packages` (see [docker-setup.md](docker-setup.md) Status) |
| Linux native client (XDG paths, PipeWire-ready capture) | Working; loopback/app audio still Windows-only |

## Partial / caveats

- **Skin is temporary.** Controls sit where Discord users expect; Banter’s own look comes later. No Discord logos.
- **Voice / calls / presence** are process-local on the server. Restart clears who is in voice and who is online.
- **One `client-session.json` per data dir.** Last login wins the next cold start when two clients share a machine and folder.
- **Stream encoder** is H.264 Constrained Baseline over WebRTC (OpenH264 software path; Media Foundation / NVENC / VA-API when the probe accepts Constrained Baseline).
- **Voice-connection subtitle** shows one plain device error (Settings still lists every device error).
- **Speaker loopback / per-app mix** for streams: Windows only.
- **GDI / naive screenshots** of the OpenGL window can come back blank or solid; prefer PrintWindow / existing capture tooling when documenting UI.
- **Email / 2FA / social login / passkeys:** designed in project notes, not the shipping local path yet.
- **Roles / kick / ban / community badges:** decided in product notes, not implemented in this foundation pass.

## Deferred (options kept)

From [foundation.md](foundation.md):

- Reactions
- Bold and italic
- Text chat inside a voice channel
- Group chats
- Roles
- Kick
- Ban

Messages are plain text. Channel and category mutations stay owner-only until roles exist.
