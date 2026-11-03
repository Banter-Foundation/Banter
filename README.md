# Banter

Local desktop chat. The window is GLFW and OpenGL. The interface is Dear ImGui from the docking branch (`dear-imgui-rs` 0.22, Dear ImGui v1.93.1 WIP docking). Accounts, servers, channels, messages, and files stay in SQLite after the app closes.

Voice membership, calls, and presence are in memory and reset when the server process stops.

## Run

Use a Visual Studio developer shell so `cl.exe` is available (SQLite and Dear ImGui compile C/C++):

```
cd D:\Projects\Banter
cargo run -p banter
```

The client listens for `127.0.0.1:3970`. If nothing is there, it starts the server in the background. Data goes to `%LOCALAPPDATA%\Banter` unless `BANTER_DATA` is set.

To run the server on its own:

```
cargo run -p banter-server
```

An optional listen address can be passed as the first argument, and that argument wins. Otherwise `BANTER_BIND` and `BANTER_PORT` are used, with `0.0.0.0` and `3970` filling whichever one is missing. When the argument and both variables are absent, the server uses `127.0.0.1:3970`. The desktop app still starts its own server on `127.0.0.1:3970`.

## Server container

Podman and Docker both run this image. The copy published for `version-1-test-1` was built with Podman.

```
podman compose up
```

Docker uses the same file:

```
docker compose up
```

Inside the container the server binds `0.0.0.0` on port `3970` (`BANTER_BIND` and `BANTER_PORT`). Account data stays on the named volume `banter-data`. Set `BANTER_PORT` to publish a different port. A secret comes from an environment variable or from `BANTER_SECRET_FILE` pointing at a mounted file. It is not copied into the image. The desktop app still looks for `127.0.0.1:3970`.

Open the client twice to try voice between two accounts. They share one server. The file `client-session.json` stores one login, so the last account to sign in is the one restored the next time a client starts. Both clients can stay signed in during the same run because each keeps its token in memory.

## Window

Before the first frame the client sets Dear ImGui `ConfigFlags::DOCKING_ENABLE`. After login the main viewport is a dockspace:

- left: server list, then the sidebar (friends or channels)
- right: members
- center: chat, with voice docked along the bottom

You can drag those panels. The layout is kept in `banter-dock.ini` in the data folder.

Push-to-talk is the V key, and it is ignored while a text field has the keyboard. The Voice dock also has a Talk button you can hold. Mute, deafen, camera, and screen share are in that same dock. If the microphone, speakers, camera, or display cannot be opened, the Voice dock shows the device error. Deafen turns mute on and restores the previous mute when you turn deafen off.

Voice channels do not have a text box. Text chat inside a voice channel is deferred.

## Tests

```
cargo test -p banter-server
cargo test -p banter
```

## Not in this build

About Me, custom status, reactions, bold and italic, text chat inside a voice channel, group chats, roles, kick, and ban. See `docs/foundation.md`.
