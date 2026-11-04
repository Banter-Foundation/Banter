# Feature status

Honest snapshot of the Discord-like **foundation** vs what still waits. The authoritative deferred list is also in [foundation.md](foundation.md). Do not implement deferred items until Owl asks.

## Works in the current build

| Area | Status |
| --- | --- |
| Register / login / logout (local username + password) | Working |
| Display name + avatar (upload crop/zoom PNG/JPG/GIF/WebP, remove → letter) | Working |
| User profiles (Discord-parity popout + customization editor: banner, theme, cosmetics, badges, bio, board cards, Activity/Wishlist) | Working — BA-73 repaired overlay/column chrome (clipped panels, Save/Cancel footer); Banter-owned procedural art; keep iterating vs Discord ref |
| Friends list + friend requests | Working — accepting (or completing a mutual request) opens that friend's DM |
| Create server, invite join, member list, server nickname | Working |
| Guild roles (@everyone, color, hoist, permissions, assign from the member menu) | Working — kick and ban stay deferred |
| Text + voice channels in categories (Manage Channels; owner has every permission; optional Lucide channel icons) | Working |
| Drawing board channels (shapes, stickies, images, files, realtime ops) | Working |
| Send / reply / edit / delete / search messages | Working |
| Reddit-style reply threading (nested replies, collapse, inline composer) | Working — text channels + DMs; `reply_to` is the parent pointer; BA-72 clarity: stronger guides, chevron fold, Reply link, distinct inline composer |
| Attach a file to a message (composer upload) | Disabled — popup **Currently Unavailable** (BA-61); existing attachments still downloadable |
| Mentions + presence | Working |
| Discord-like temporary skin (rail, sidebar, chat, members, modals) | Working — **not** final Banter branding |
| Chat rows (avatar, hover toolbar, bottom-anchored history; threaded tree layout) | Working |
| Right-click context menus (messages, members/friends/DMs, channels) | Working — foundation actions only; no kick/ban/profiles |
| Drop-in voice: mute, deafen, PTT, connection panel | Working (SFU rewrite outbound SSRC/PT on forward; open-mic Opus timeline) |
| Camera on/off with mirrored local RGBA preview (~720p prefer; device MJPEG OK) | Working (device-dependent); wire is WebRTC H.264; remotes receive after RID map + forward SSRC rewrite |
| Screen share Go Live + quality / source switching | Working (WebRTC H.264 SFU/P2P; remotes receive after RID map + forward SSRC rewrite) |
| 1:1 DMs + start a call | Working — phone icon opens a Discord-style call stage (ringing avatar / two tiles when active) |
| User Settings: My Account (display name, About Me, custom status), Voice & Video, Developer (Local/Remote API, persisted) in a dedicated Banter Settings OS window | Working |
| Embedded + standalone server, PostgreSQL data | Working |
| Containerized `banter-server` (`ghcr.io/snowedinowl/banter`) | Image/Dockerfile ready; **GHCR publish + release auto-deploy blocked** until classic PAT has `write:packages` (see [docker-setup.md](docker-setup.md) Status) |
| Linux native client (XDG paths, PipeWire-ready capture) | Working; loopback/app audio still Windows-only |
| Hidden `/debugnow` composer command | Working — opens a server debug window (logs and live stats). Not listed in the UI and not stored as a message. Text channels and DMs. |

## Partial / caveats

- **Skin is temporary.** Controls sit where Discord users expect; Banter’s own look comes later. No Discord logos.
- **Voice / calls / presence** are process-local on the server. Restart clears who is in voice and who is online.
- **One `client-session.json` per data dir** when Stay signed in is checked. Last login wins the next cold start when two clients share a machine and folder. Unchecked sign-in does not write (and clears) that file; Log Out always clears it.
- **Stream encoder** is H.264 Constrained Baseline over WebRTC (OpenH264 software path; Media Foundation / NVENC / VA-API when the probe accepts Constrained Baseline).
- **Voice-connection subtitle** shows one plain device error (Settings still lists every device error).
- **Speaker loopback / per-app mix** for streams: Windows only.
- **GDI / naive screenshots** of the OpenGL window can come back blank or solid; prefer PrintWindow / existing capture tooling when documenting UI.
- **Email / 2FA / social login / passkeys:** designed in project notes, not the shipping local path yet.
- **Kick / ban / community badges:** decided in product notes, not implemented in this foundation pass. Guild roles (color, hoist, and the four permission bits) are in this build.

## Text formatting (BFMD)

| Piece | Status |
| --- | --- |
| BFMD parse / serialize (v1 commands) | Working — `crates/banter/src/bfmd.rs` + tests |
| Persist BFMD in message `body` | Working — plain text unchanged |
| Message-row ImGui render (runs) | Working — bold/italic/underline/strike/size/color/bg/hr |
| Composer B/I/U/S toolbar | Working — toggles whole draft (ImGui selection limited) |

Authoritative language: [bfmd.md](bfmd.md). Not CommonMark.

## Deferred (options kept)

From [foundation.md](foundation.md):

- Reactions
- Text chat inside a voice channel
- Group chats
- Kick
- Ban

Message bodies are BFMD source (plain text is valid). Channel and category changes require Manage Channels. Invites require Create Invite.
