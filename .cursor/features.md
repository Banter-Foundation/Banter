# Features

## Foundation

- [*] Unique username, display name, avatar, and password. An avatar can be a PNG, JPG, GIF, or WebP, cropped and zoomed to a circle, and removed back to the letter. Sign-in has **Stay signed in**: checked writes `client-session.json` for relaunch; unchecked keeps the token in memory only and clears any saved file. Log Out always clears the saved session.
- [*] User profiles (Discord parity): view banner, themed card, avatar decorations, styled display name, pronouns, custom status, About Me, Member Since, connections, and Board/Activity/Wishlist widgets; edit with a Discord-style customization editor (nameplate, decorations, display-name style, theme & banner, profile effect/frame, widgets). Includes badge row, Activity/Wishlist tabs, Discord slot chrome, board game cards, and Save/Cancel for banner/avatar; custom status on presence; Main vs server profile switch; clickable connections. Profile view and customization editor use dedicated overlay windows with clipped identity / left / preview / board columns so panels no longer overflow or collide (BA-73). Open from chat, the member list, friends, or the user-bar avatar.
- [*] Friend requests and a friends list. Accepting a request (or completing a mutual add) opens that friend's DM.
- [*] Create a server, join by invite, member list, and server nickname
- [*] Nested server-rail folders with custom hex colors (BA-79): create/rename/delete, nest servers and subfolders, collapse/expand (persisted), color via hex + RGB sliders; per-user Postgres layout
- [*] Text and voice channels in categories
- [*] Send, reply, edit, delete, and search. Text channels and DMs use Reddit-style nested reply threads (strong indent/guides, chevron collapse, always-visible Reply link, accented inline composer vs top-level bar). Leaf rows keep Discord-like avatar, time, and hover toolbar polish. Message bodies use **BFMD** (Better Formatted Markdown) for styles: bold, italic, underline, strikethrough, size, color, highlight, font, align, heading, and horizontal rule. Plain text remains valid BFMD. Composer toolbar applies B/I/U/S; message rows render styled runs in ImGui.
- [*] Attach a file from the composer is temporarily disabled (BA-61): the attach control opens a Banter dialog that says exactly `Currently Unavailable` instead of uploading.
- [*] Discord-style right-click menus on messages, members/friends/DMs, and channels (foundation actions only; kick/ban stay deferred)
- [*] Drop-in voice with mute, deafen, push-to-talk, camera, and screen share. Voice & Video settings and stream quality drive the engine and are saved on this PC. Stream audio mix sources are grouped under Audio channels and Apps.
- [*] 1:1 direct messages that can start a call. Starting or accepting a call opens a Discord-style call stage in that DM (large avatar while ringing, two tiles when active, shared call controls).
- [*] Presence and mentions
- [*] GLFW + OpenGL window with Dear ImGui docking
- [*] User Settings in a dedicated Banter Settings GLFW window (My Account, Voice & Video, Developer); main shell stays usable; Escape / close returns focus cleanly. My Account edits display name, About Me, and custom status. Custom status shows under your name and on member rows.
- [*] Guild roles: @everyone plus extra roles with a color, optional hoist, and permission bits (administrator, manage channels, manage roles, create invite). Server menu edits roles. Member context menu assigns them. Owner has every permission. Kick and ban stay deferred.
- [*] Developer settings: Local (`127.0.0.1:3970`) or Remote host, persisted in `dev-server.json`. Client startup does not require local unless logged in on Local or the user picks Local; missing local asks for remote instead of hard-failing.
- [*] F3 profiler: FPS, RSS, HTTP/WebSocket bytes, frame sections, expensive ops
- [*] Hidden composer command `/debugnow` (text channels and DMs) opens a server debug window with logs and live stats. It is not listed in the UI and is not sent as a message.
- [*] Display settings: VSync, fullscreen, window resolution (saved in video.json). With VSync on, the client UI matches the current monitor’s refresh rate and reapplies when the window moves or the display mode changes (not stream capture FPS).

## Deferred

Owl will decide later. Do not delete these.

- Reactions
- Text chat inside a voice channel
- Group chats
- Kick
- Ban
