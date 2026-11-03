# Banter foundation

Chosen 2026-11-02. This is the slice the desktop app and local server implement.

For a longer status table (what works, caveats, deferred), see [feature-status.md](feature-status.md). For stack and paths, see [overview.md](overview.md).

## In this build

- Unique username, display name, avatar, and password
- Friend requests and a friends list
- Create a server, join by invite, member list, and a server nickname
- Text and voice channels in categories
- Send, reply, edit, delete, attach a file, and search
- Drop-in voice with mute, deafen, push-to-talk, camera, and screen share
- Voice media is Opus and simulcast H.264 over WebRTC. See [streaming.md](streaming.md)
- 1:1 direct messages that can start a call
- Presence and mentions

## Deferred

Still options. Not implemented, and not removed from the plan. Owl will decide later.

- About Me
- Custom status
- Reactions
- Bold and italic
- Text chat inside a voice channel
- Group chats
- Roles
- Kick
- Ban

Messages are plain text. Channel and category changes are owner-only until roles exist. Voice-channel text is rejected by the server.
