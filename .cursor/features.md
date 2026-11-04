# Features

## Foundation

- [*] Unique username, display name, avatar, and password. An avatar can be a PNG, JPG, GIF, or WebP, cropped and zoomed to a circle, and removed back to the letter.
- [*] User profiles (Discord parity): view banner, themed card, avatar decorations, styled display name, pronouns, custom status, About Me, Member Since, connections, and Board/Activity/Wishlist widgets; edit with a Discord-style customization editor (nameplate, decorations, display-name style, theme & banner, profile effect/frame, widgets). Open from chat, the member list, friends, or the user-bar avatar.
- [*] Friend requests and a friends list. Accepting a request (or completing a mutual add) opens that friend's DM.
- [*] Create a server, join by invite, member list, and server nickname
- [*] Text and voice channels in categories
- [*] Send, reply, edit, delete, attach a file, and search. Rows show a round avatar, a local time, grouped follow-ups, and a hover toolbar.
- [*] Discord-style right-click menus on messages, members/friends/DMs, and channels (foundation actions only; kick/ban stay deferred)
- [*] Drop-in voice with mute, deafen, push-to-talk, camera, and screen share. Voice & Video settings and stream quality drive the engine and are saved on this PC.
- [*] 1:1 direct messages that can start a call. Starting or accepting a call opens a Discord-style call stage in that DM (large avatar while ringing, two tiles when active, shared call controls).
- [*] Presence and mentions
- [*] GLFW + OpenGL window with Dear ImGui docking
- [*] Developer settings: Local (`127.0.0.1:3970`) or Remote host, persisted in `dev-server.json`. Client startup does not require local unless logged in on Local or the user picks Local; missing local asks for remote instead of hard-failing.
- [*] F3 profiler: FPS, RSS, HTTP/WebSocket bytes, frame sections, expensive ops
- [*] Display settings: VSync, fullscreen, window resolution (saved in video.json). With VSync on, the client UI matches the current monitor’s refresh rate and reapplies when the window moves or the display mode changes (not stream capture FPS).

## Deferred

Owl will decide later. Do not delete these.

- Reactions
- Bold and italic
- Text chat inside a voice channel
- Group chats
- Roles
- Kick
- Ban
