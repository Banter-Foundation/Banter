# Mods

Luau that runs on the client of the person who enabled it. Shared sandbox, permissions, the render function, and export rules are in [Modding](modding.md).

A mod changes the local client: layout within the limits below, extra UI, and optional actions as the signed-in user.

## Install

- Folder: `<Banter data dir>/mods/<folder>/`
- Required file: `banter.mod.json`
- Entry file: path in `entry`, default `main.luau`, inside the folder
- Other files: `.luau`, `.json`, `.png`, `.webp` only. Each file at most 256 KB. The folder at most 2 MB and 64 files.
- The client scans the mods directory at startup and when the directory changes.
- A bad manifest is listed in Settings → Mods as failed, with the reason, and is not started.
- The user enables a mod explicitly. Enable creates the Luau state and runs the entry. Disable destroys the state and removes its UI and shell overrides.

`banter.mod.json`:

```json
{
  "id": "clock_bar",
  "name": "Clock Bar",
  "version": "1.0.0",
  "banter": "1",
  "entry": "main.luau",
  "permissions": ["log", "storage", "ui", "read_screen"],
  "memory_mb": 8
}
```

- `id` must match the identifier rule and must be unique among enabled mods.
- `name` is 1–32 characters and is the Settings label.
- `permissions` is the checklist. Unknown names make the manifest fail. Each name turns on the APIs below. Any other host call returns nil.
- `memory_mb` is how much memory this mod requires, in megabytes. It is an integer of at least 1. Omitted means 8. The host caps that mod's Luau state at this size. Going past it stops the mod with a hard error `memory_limit`.

Permissions:

- `log` — `banter.log`. Prompt line: “Write lines to this mod’s log.”
- `storage` — `banter.storage`. Prompt line: “Save data for this mod on this computer.”
- `ui` — `banter.ui` and `banter.shell`. Prompt line: “Add buttons, panels, and layout changes in Banter.”
- `read_screen` — `banter.screen` and `banter.events`. Prompt line: “Read the server, channel, messages, and members you already have open.”
- `messages` — `banter.user` message calls. Prompt line: “Send, edit, and delete messages as you.”
- `voice` — `banter.user` voice calls. Prompt line: “Join and leave voice as you.”
- `presence` — `banter.user.set_custom_status`. Prompt line: “Change your custom status.”
- `io` — Luau `io`. Second warning. Prompt line on that dialog: “This mod can read and write files on this computer.”
- `os` — Luau `os`. Second warning. Prompt line on that dialog: “This mod can run programs, read environment variables, and use other operating system functions.”

The first dialog lists every checked permission except `io` and `os`, and it lists “This mod requires N MB of memory.” with N taken from `memory_mb`. The second dialog is the invasive warning and lists only `io` and `os`.

When `memory_mb` is greater than 150, Allow on the dialogs above opens one more dialog and does not load the mod yet:

- Title: the mod name
- Body: “This mod requires N MB of memory. That is more than 150 MB.”
- Buttons: Allow and Cancel. Cancel leaves the mod disabled. Every dialog that opened must be allowed.

## Process rate

Each mod has its own rate. Callbacks from one mod do not spend another mod's budget.

The rate is locked at 1,000,000 Luau instructions and 50 ms of wall time per callback. The manifest cannot change either number. Past either limit the host records `instruction_limit` or `time_limit` and stops that mod.

Settings → Mods shows **Unlock process rate** on each mod. It is off. Turning it on applies only to that mod on this computer, and it is stored in the Banter data directory, not in the mod package. With it on, that mod is not stopped for `instruction_limit` or `time_limit`. Other mods stay locked. Turning it off again applies on the next callback.

## Load order

Enabled mods load and run in one list. Settings → Mods shows that list. The user can move a mod up or down. The list is saved in the Banter data directory.

Until the user moves a mod, the order is the mod id, ascending, compared byte by byte. A newly enabled mod is placed at the end. Disabling a mod takes it out of the list. Enabling it again places it at the end.

The host uses that same order, first to last, for all of the following:

- creating the Luau state and running the entry
- calling each render function during a frame
- delivering one event to each mod that subscribed to it
- applying `banter.shell.set` and `banter.theme.set`, so the last mod in the list wins when two mods set the same field

## Lifecycle

1. Host creates an empty Luau state and injects `banter`, in load order.
2. Host runs the entry. The entry registers `banter.render` and event handlers. It should return immediately. A Luau error here is a hard error and stops the mod.
3. Every frame, the host calls each render function in load order. Unclosed regions become soft errors and are closed by the host. A Luau error in render or in an event callback is a hard error and stops the mod.
4. Disable destroys the state and removes its UI, theme patch, and shell overrides.

At most 32 mods are enabled. Enabling a 33rd throws `limit` in the Settings UI.

## `banter.log(level, message)`

- Permission: `log`. Returns nil when `log` is not checked.
- `level`: `debug` | `info` | `warn` | `error`
- `message`: string, at most 2000 characters
- Writes to the client mod log for that id. It does not write the server debug log.

## `banter.storage`

Permission: `storage`. Returns nil when `storage` is not checked. Local to this machine and this mod id. Survives restart. Not uploaded.

- `get(key)` → value or `nil`
- `set(key, value)` → nothing. `value` encoded JSON at most 64 KB. Total per mod 1 MB. Past that, `limit`.
- `delete(key)` → nothing
- `keys()` → array of strings

## `banter.screen`

Permission: `read_screen`. Returns nil when `read_screen` is not checked. Reads data the client already has. It does not fetch new pages by itself.

- `now()` →

```lua
{
  user = { id = 1, username = "ada", display_name = "Ada" },
  view = "home", -- "home" | "friends" | "channel" | "dm" | "settings"
  server = nil, -- or { id, name }
  channel = nil, -- or { id, server_id, category_id, name, kind, icon }
  dm = nil, -- or { id, peer = { id, username, display_name } }
  members_visible = true,
}
```

- `messages()` → array of the messages currently loaded in the open text channel or DM, oldest first, at most the client’s current page. Each message matches the server `Message` object: `id`, `author_id`, `author_username`, `author_display_name`, `author_nickname`, `body`, `reply_to`, `created_at`, `edited_at`, `attachments` (`{ id, filename, byte_size }`). Empty array when the open view has no messages.
- `members()` → array of members of the open server, or empty on Home. Each: `user_id`, `username`, `display_name`, `nickname`, `is_owner`, `roles` (`{ id, name, color, position, hoist, permissions, everyone }`).
- `voice()` → `{ channel_id, members = { { user_id, muted, deafened, transmitting, camera, screen } } }` or `nil` when the user is not in voice.

## `banter.events.on(name, fn)`

Permission: `read_screen`. `on` returns nil when `read_screen` is not checked, and the handler is not registered. `fn` is called on the client thread between frames. Events are the ones the signed-in user already received.

- `ready` — `{}` after the entry returns and the screen snapshot exists
- `message_created` — `{ target = { type = "channel"|"direct", id = <n> }, message = <Message> }`
- `message_edited` — same shape
- `message_deleted` — `{ target, message_id }`
- `presence` — `{ user_id, state = "online"|"idle"|"offline", custom_status }`
- `voice` — `{ channel_id, members }` using the `voice()` member shape
- `screen` — `{}` when `now()` would change

`banter.events.off(name, fn)` removes one handler.

## `banter.shell.set(patch)` and `banter.shell.clear()`

Permission: `ui`. `set` returns nil when `ui` is not checked. Local layout only. Cleared on disable.

`patch` fields, each optional:

- `sidebar_width` — number, clamped to 180–400
- `members_visible` — boolean

Two mods may both set these. The host applies them in load order. The last mod in that order wins a field. `clear()` removes this mod’s patch.

## `banter.theme`

Permission: `ui`. `set` returns nil when `ui` is not checked. A mod's widgets start on the Banter theme. `set` changes that default. `shell` changes the Banter window itself. Both clear when the mod is disabled.

`banter.theme.get()` → the token table this mod is using right now. Colors are numbers, `0xRRGGBB` or `0xAARRGGBB`.

`banter.theme.set(patch)` changes this mod's default theme. Omitted tokens stay on the Banter theme. `banter.theme.reset()` puts this mod's defaults back on the Banter theme. Tokens:

- `text`, `muted`, `heading`, `danger`, `link`, `accent`
- `background`, `input_bg`, `button`, `button_hover`, `button_text`

`banter.theme.shell(patch)` changes the Banter window. The host applies every enabled mod's shell patch in load order. The last mod that sets a token wins that token. `banter.theme.clear_shell()` removes this mod's shell patch. Shell tokens:

- `rail_bg`, `sidebar_bg`, `stage_bg`, `panel_bg`, `composer_bg`, `input_bg`
- `text`, `muted`, `accent`, `danger`
- `row_hover`, `row_selected`

An unknown token throws `invalid` when `ui` is checked.

A widget call may pass a `theme` table for that element only. Keys are `color`, `text_color`, `background`, and `hover`. A key that is left out uses the mod's current theme. This does not change the default theme and does not change the Banter window.

```lua
banter.theme.set({ accent = 0x23A55A, button = 0x248046 })
banter.theme.shell({ sidebar_bg = 0x1E1F22 })

banter.render(function()
  if banter.ui.begin("composer.toolbar") then
    banter.ui.text("Clock", "muted")
    banter.ui.button("Ping", "primary", { background = 0x5865F2, text_color = 0xFFFFFF })
    banter.ui.close()
  end
end)
```

## `banter.ui.stage` for mods

A mod claims a built-in kind by calling `banter.ui.begin("stage", kind)` with `text`, `voice`, or `board`. It may not invent a kind. When several enabled mods claim the same kind, the earliest mod in the load order paints the stage. The user can pick “Banter” to keep the built-in stage.

## `banter.user`

Calls go out as the signed-in user, with that user’s real permissions, and only for permissions this mod checked. The token stays inside the client. Each function below returns nil when its permission is not checked.

- `send_message(target, body, reply_to)` → `Message`. Permission: `messages`. `target` is `{ type = "channel"|"direct", id }`. `reply_to` may be `nil`. `body` is BFMD, at most 2000 characters.
- `edit_message(id, body)` → `Message`. Permission: `messages`. Own messages only.
- `delete_message(id)` → nothing on success. Permission: `messages`. Own messages only.
- `join_voice(channel_id)` → nothing on success. Permission: `voice`.
- `leave_voice()` → nothing on success. Permission: `voice`.
- `set_custom_status(text)` → nothing on success. Permission: `presence`. `text` at most 128 characters. Empty string clears it.
- `layout(server_id)` → the server home payload the client would get from `GET /v1/servers/{id}/layout`. Permission: `read_screen`.
- `open_messages(target, before, limit)` → `Message` array. Permission: `read_screen`. `before` is a message id or `nil`. `limit` is 1–100.

Failures use the server `code` (`not_author`, `not_text_channel`, `forbidden` mapped from `not_owner` / `not_member`, and the rest of the existing codes).

## Example

```lua
local label = "Home"

banter.render(function()
  local screen = banter.screen.now()
  if screen and screen.channel then
    label = screen.channel.name
  end
  if banter.ui.begin("stage.header.actions") then
    banter.ui.text(label, "muted")
    banter.ui.close()
  end
end)

banter.events.on("message_created", function(event)
  banter.log("info", event.message.body)
end)
```
