# Banter foundation

Chosen 2026-11-02. This is the slice the desktop app and local server implement.

For a longer status table (what works, caveats, deferred), see [feature-status.md](feature-status.md). For stack and paths, see [overview.md](overview.md).

## In this build

- Unique username, display name, avatar, and password
- Friend requests and a friends list
- Create a server, join by invite, member list, and a server nickname
- Text and voice channels in categories
- Send, reply, edit, delete, attach a file, and search
- Chat text formatting via **BFMD** (style-command language adapted from Sickle — not CommonMark). See [bfmd.md](bfmd.md). Plain text bodies stay valid.
- Drop-in voice with mute, deafen, push-to-talk, camera, and screen share
- Voice media is Opus and simulcast H.264 over WebRTC. See [streaming.md](streaming.md)
- 1:1 direct messages that can start a call
- Presence and mentions
- Discord-parity user profiles: banner, theme, nameplate, avatar decorations, display-name style, profile effect/frame, About Me, pronouns, custom status, Member Since, connections, badge row, Board game cards, and user-authored Activity / Wishlist rows
- My Account edits About Me and custom status. Custom status shows under your name in the user bar and on member rows.
- Guild roles: one @everyone role per server, extra roles with a color and optional hoist, and permission bits for administrator, manage channels, manage roles, and create invite. The server menu edits roles. A member's context menu assigns them. The owner has every permission. Kick and ban stay deferred.

## Deferred

Still options. Not implemented, and not removed from the plan. Owl will decide later.

- Reactions
- Text chat inside a voice channel
- Group chats
- Kick
- Ban

Message bodies are BFMD source (plain text is valid BFMD). Creating a channel or category requires Manage Channels. Creating an invite requires Create Invite. Voice-channel text is rejected by the server.
