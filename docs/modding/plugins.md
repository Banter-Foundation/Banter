# Plugins

Luau installed on one server. The author sets whether members install a client half. A server-only plugin never asks. A client script runs only after that member accepts that version. Shared sandbox, the render function, and export rules are in [Modding](modding.md). The client UI calls are the same ones documented for [Mods](mods.md).

A plugin adds server behavior for one guild, and may add client UI for members who accept it. Examples: a command that posts a message, a channel kind with its own stage, a small game whose rules live on the server.

## Package

Zip, at most 2 MB, at most 64 files. Allowed suffixes: `.luau`, `.json`, `.png`, `.webp`. Each file at most 256 KB. Paths are relative, with no `..` and no absolute path. Required: `banter.plugin.json`.

```json
{
  "id": "chess",
  "name": "Chess",
  "version": "1.2.0",
  "banter": "1",
  "server_entry": "server/main.luau",
  "client": "required",
  "client_entry": "client/main.luau",
  "permissions": ["storage", "messages", "channels", "members_read", "events", "sync", "commands"],
  "client_permissions": ["ui", "read_screen", "sync"],
  "channel_kinds": [
    { "name": "match", "label": "Match", "icon": "swords" }
  ],
  "commands": [
    {
      "name": "chess",
      "description": "Start a game",
      "options": [
        { "name": "opponent", "type": "user", "required": true, "description": "Who to play" }
      ]
    }
  ]
}
```

- `client` is required. It is `none`, `optional`, or `required`.
  - `none` is server only. `client_entry` must be omitted. `client_permissions` must be omitted. The package has no client files. Members are not prompted, and the client does not download a bundle.
  - `optional` ships a client bundle. `client_entry` is required. Members may accept or skip it. Server commands and other server behavior keep working when they skip it.
  - `required` ships a client bundle. `client_entry` is required. The server script still runs for the whole guild. Custom stages stay on the placeholder until that member accepts the client bundle.
- A package that pairs `none` with `client_entry`, or `optional` / `required` without `client_entry`, fails to install.
- `server_entry` is required when `client` is `none`. It may be omitted when `client` is `optional` or `required`, and then only the client script runs.
- `channel_kinds[].name` follows the package id rule. The stored channel kind is `<plugin id>.<name>`, for example `chess.match`. `label` is 1–32 characters. `icon` is an optional Lucide id, the same set channels already use.
- `commands` max 32. `name` follows the package id rule and is unique per server across plugins and bots. `description` is 1–100 characters. `options` max 8.
- Option `type`: `string` | `integer` | `number` | `boolean` | `user` | `channel`. `string` values are at most 100 characters. `integer` is a whole number. `user` and `channel` are ids in this server.
- `permissions` is the server-script checklist. `client_permissions` is the client-script checklist. Unknown names make the package fail. A server call whose permission is missing returns nil. A client call whose permission is missing returns nil.

Server permissions: `storage`, `messages`, `channels`, `members_read`, `events`, `sync`, `commands`, `io`, `os`.

Client permissions: `log`, `ui`, `read_screen`, `sync`, `io`, `os`.

`io` and `os` use the second warning. The admin sees it for the server list. Each member sees it for the client list when they accept the plugin. Server prompt lines say “this server”. Client prompt lines say “this computer” and “This mod”, using the plugin name.

## Install and life cycle

Admin routes, caller is owner or has `manage_plugins` (a new permission bit, same grant rules as the existing role bits):

- `GET /v1/servers/{id}/plugins` → array of installs: `{ id, name, version, enabled, client, has_server, has_client, channel_kinds, commands }`. `client` is `none`, `optional`, or `required`. `has_client` is false when `client` is `none`.
- `POST /v1/servers/{id}/plugins` with `Content-Type: application/zip` body. Replaces an existing install with the same `id` only when `version` is greater. Same or lower version returns `409` `conflict`.
- `PATCH /v1/servers/{id}/plugins/{plugin_id}` body `{ "enabled": true|false }`
- `DELETE /v1/servers/{id}/plugins/{plugin_id}` disables, unloads, and deletes the package. Channels of its kinds remain and show the placeholder stage.

On enable, the server creates one Luau state for that server and plugin and runs `server_entry`. On disable or server stop, it destroys the state. A Luau error in the server entry or in a server callback is a hard error: that server state stops, and the error is listed on the install until an admin enables it again. At most 16 enabled plugins per server. A hard error in the client render stops only that member’s client state. Soft errors from an unclosed UI stay on the client and leave the server script running.

Member routes:

- `GET /v1/servers/{id}/plugins` returns the same metadata for any member. It does not return script source.
- `POST /v1/servers/{id}/plugins/{plugin_id}/opt-in` body `{ "version": "1.2.0" }`. Stores the accept for this user, server, plugin id, and version. The route returns `404` `not_found` when `client` is `none`.
- `DELETE /v1/servers/{id}/plugins/{plugin_id}/opt-in` forgets the accept and the client unloads that state. The route returns `404` `not_found` when `client` is `none`.
- `GET /v1/servers/{id}/plugins/{plugin_id}/client` returns the client files only when `client` is `optional` or `required` and the caller’s stored accept matches the installed version. Otherwise `403` `forbidden`. When `client` is `none`, the route returns `404` `not_found`.

The client prompts only when `client` is `optional` or `required` and this member has not accepted that version. The prompt shows the name, the version, and “This can draw UI inside this server and send moves to the server.” Actions: Accept, Not now. For `optional`, Not now leaves server commands working and leaves custom stages on a placeholder that offers Accept again. For `required`, Not now leaves those stages on a placeholder that says the plugin needs a client install, and server commands still run. User Settings for that server lists `optional` and `required` plugins with Accept / Remove. A `none` plugin is listed there as “Server only” and has no Accept action.

## Channel kinds

Creating a channel of kind `chess.match` requires the chess plugin to be enabled on that server. The kind string is stored in `channels.kind`. A later schema adds `channels.plugin_id`. Built-in kinds stay `text`, `voice`, and `board`.

Members of a `none` plugin see its custom channels in the sidebar and a stage that says the plugin is server only. There is no Accept action. Members who have not accepted an `optional` or `required` client bundle see the channel in the sidebar and a stage that names the plugin and offers Accept. Members who accepted it paint that stage from `banter.ui.begin("stage", "chess.match")` inside the plugin’s render function. A plugin cannot claim `text`, `voice`, or `board`, or another plugin’s kind.

A plugin may delete only channels it created. The server records that on the channel row (`created_by_plugin`).

## Commands

The composer lists plugin commands for the open server to every member, whether or not they opted in. Choosing one sends:

`POST /v1/commands`

```json
{
  "server_id": 1,
  "channel_id": 2,
  "name": "chess",
  "options": { "opponent": 9 }
}
```

The server checks that the caller is a member, the channel is in that server, and the option types match. It then runs the plugin’s `command` event. The handler’s return value is either `nil` or `{ body = "<BFMD>" }`. A body becomes a plugin-authored message in that channel. The HTTP response is that message, or `204` when the handler returns `nil`.

Plugin-authored messages need a later schema change: `messages.author_kind` (`user` or `plugin`) and nullable `messages.plugin_id`. User and bot messages keep `author_id`. Plugin messages leave `author_id` null and set `plugin_id`. The client shows the plugin name and a Plugin badge.

## Server host API

`banter.server` is `{ id, name }`. `banter.plugin` is `{ id, name, version, client }`. `client` is `none`, `optional`, or `required`.

### `banter.storage`

Permission `storage`. Returns nil when `storage` is not checked. Postgres, scoped to `(server_id, plugin_id)`.

- `get(key)` → value or `nil`
- `set(key, value)` → nothing. Encoded value at most 256 KB. Total per install 16 MB.
- `delete(key)` → nothing
- `keys()` → array

### `banter.messages`

Permission `messages`. Each function returns nil when `messages` is not checked.

- `send({ channel_id, body, reply_to })` → message. The author is this plugin. `body` max 2000. The channel must be `text` or a custom kind this plugin owns. Voice and board reject with `not_text_channel`.
- `edit(id, body)` → message. Only messages this plugin sent.
- `delete(id)` → nothing. Only messages this plugin sent.
- `list(channel_id, before, limit)` → array, `limit` 1–100.

### `banter.channels`

Permission `channels`. Each function returns nil when `channels` is not checked.

- `list()` → the layout object for this server
- `get(id)` → channel or `nil`
- `create({ category_id, name, kind, icon })` → channel. `kind` is `text`, `voice`, `board`, or one of this plugin’s full kind ids.
- `delete(id)` → nothing. Only channels this plugin created.

### `banter.members`

Permission `members_read`. Each function returns nil when `members_read` is not checked.

- `list()` → member array, same fields as `banter.screen.members` in a mod
- `get(user_id)` → member or `nil`

### `banter.events.on(name, fn)`

Permission `events` for the membership, message, and voice handlers. `command` needs `commands`. `on` returns nil and does not register when the matching permission is not checked. `rpc` is registered through `banter.rpc.handle`.

- `load` — `{}` after the entry returns
- `unload` — `{}` before the state is destroyed
- `message_created` — `{ channel_id, message }` for text channels in this server
- `message_edited` — same
- `message_deleted` — `{ channel_id, message_id }`
- `member_join` — `{ user_id }`
- `member_leave` — `{ user_id }`
- `voice_join` — `{ channel_id, user_id }`
- `voice_leave` — `{ channel_id, user_id }`
- `command` — `{ channel_id, user_id, name, options }`

DMs are not delivered to plugins.

### `banter.sync`

Permission `sync`. Each function returns nil when `sync` is not checked. State is a JSON-like value, at most 64 KB, one current value per channel.

- `set(channel_id, state)` → nothing. The channel must belong to this server. Pushes `PluginSync` to opted-in members who can view the channel.
- `get(channel_id)` → state or `nil`
- `clear(channel_id)` → nothing. Pushes a sync with `state: null`.

Websocket server event:

```json
{
  "kind": "plugin_sync",
  "server_id": 1,
  "plugin_id": "chess",
  "channel_id": 2,
  "state": {}
}
```

### `banter.rpc.handle(name, fn)`

Permission `sync`. `handle` returns nil when `sync` is not checked. `name` follows the package id rule. At most 32 handlers.

`fn` receives `{ user_id, channel_id, payload }` and returns a JSON-like value at most 64 KB. `payload` from the client is at most 16 KB. The caller must be an opted-in member who can view the channel. The handler’s 50 ms budget applies.

Client signal:

```json
{
  "kind": "plugin_rpc",
  "plugin_id": "chess",
  "channel_id": 2,
  "name": "move",
  "request_id": "c5b0c2e0-7c1a-4e0a-9a1f-0a0a0a0a0a0a",
  "payload": {}
}
```

Server event back to that socket only:

```json
{
  "kind": "plugin_rpc_result",
  "request_id": "c5b0c2e0-7c1a-4e0a-9a1f-0a0a0a0a0a0a",
  "ok": true,
  "result": {}
}
```

On handler failure, `ok` is false and `error` is the code string. `result` is omitted.

## Client host API

This section applies when `client` is `optional` or `required`. A `none` plugin has no client state. The script runs only after opt-in, only while that server is in the client’s server list, and only for the accepted version. Each API returns nil unless its name is in `client_permissions`.

- `banter.log(level, message)` — permission `log`
- `banter.plugin` — `{ id, name, version, server_id }`, always present
- `banter.screen.now()`, `messages()`, `members()`, `voice()` — permission `read_screen`. Same shapes as mods, and `now().server.id` is this plugin’s server. When the user is looking at another server, `channel` and `dm` are `nil` and `view` is `other`.
- `banter.events` — permission `read_screen`. The mod event names, filtered to this server’s channels and members.
- `banter.ui` — permission `ui`. Same render function as mods. Regions the plugin draws are labeled with the plugin name. `begin("stage", kind)` only accepts this plugin’s own kinds. Widget calls may take the same `theme` table as mods. `banter.theme.set` and `banter.theme.reset` change the default theme for this plugin’s widgets only. `banter.theme.shell` is nil.
- `banter.sync.on(fn)` — permission `sync`. `fn(channel_id, state)` when a `plugin_sync` arrives, including the initial state when the stage opens
- `banter.sync.current(channel_id)` → last state or `nil`. Permission `sync`.
- `banter.rpc.call(name, channel_id, payload)` yields until `plugin_rpc_result`. Permission `sync`. Returns the result, or throws the error code when `sync` is checked.
- `io` and `os` — permissions `io` and `os`. Same second warning as a mod, shown to the member.

Plugin client scripts have no `banter.user` and no `banter.shell`. Those functions are nil. Durable shared state goes through `sync` or `rpc`.

## Example

Server:

```lua
local games = {}

banter.rpc.handle("move", function(req)
  local game = games[req.channel_id]
  game.last = req.payload
  banter.sync.set(req.channel_id, game)
  return { ok = true }
end)

banter.events.on("command", function(cmd)
  if cmd.name ~= "chess" then
    return
  end
  local game = { white = cmd.user_id, black = cmd.options.opponent, turn = "white" }
  games[cmd.channel_id] = game
  banter.sync.set(cmd.channel_id, game)
  return { body = "Game started." }
end)
```

Client:

```lua
banter.render(function()
  local screen = banter.screen.now()
  if not screen or not screen.channel then
    return
  end
  if not banter.ui.begin("stage", "chess.match") then
    return
  end
  local state = banter.sync.current(screen.channel.id)
  local label = "Waiting"
  if state then
    label = state.turn
  end
  banter.ui.text(label, "heading")
  if banter.ui.button("Move") then
    banter.rpc.call("move", screen.channel.id, { from = "e2", to = "e4" })
  end
  banter.ui.close()
end)
```
