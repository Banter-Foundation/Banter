# Banter chat commands (BA-104)

Discord-like composer commands with a **per-server prefix** (default `/`), a popup list, Tab/Enter to complete or run, and an extensible client registry. Guild admins can change the prefix and define **aliases** (multiple names → one handler).

## Composer UX

1. Type the server’s command prefix at the start of the composer (DMs and Home use `/`).
2. A floating `##command-popup` lists matching non-hidden commands above the bar.
3. **↑ / ↓** move the highlight. **Tab** completes `{prefix}{name} `. **Enter** runs the highlighted command (or the fully typed one). **Escape** clears the draft while the popup is open.
4. Autosuggest (BA-103) should yield while the buffer starts with the command prefix.

## Prefix and aliases (per guild)

| Setting | Storage | Who can change |
| --- | --- | --- |
| `command_prefix` | `servers.command_prefix` (default `/`) | Manage Channels (or Administrator / owner) |
| Aliases | `server_command_aliases (server_id, alias, target)` | same |

API:

- `GET /v1/servers/{id}/commands` → `{ "prefix": "/", "aliases": [{ "alias": "s", "target": "shrug" }] }`
- `PUT /v1/servers/{id}/commands` → same shape (replaces alias list)

UI: server header menu → **Chat Commands** (Manage Channels).

Rules:

- Prefix: 1–8 characters, no whitespace.
- Alias / target names: lowercase `a-z`, digits, `_`, `-`, 1–32 chars; alias ≠ target; unique aliases.
- Clients apply fetched settings into `DefaultCommandRegistry` when the shell loads a server; DMs/Home reset to `/` with no guild aliases.
- Hidden `/debugnow` still works as a whole message even if the guild prefix is not `/`.

## Built-in commands (v1)

| Name | Aliases | Behavior |
| --- | --- | --- |
| `me` | — | Sends a BFMD italic action: `<{i:t}>{display} {args}` |
| `shrug` | `shruggie` | Sends message + `¯\_(ツ)_/¯` |
| `tableflip` | — | Sends message + tableflip face |
| `mute` | — | Toggles local mute (`MuteMemory`) |
| `deafen` | — | Toggles local deafen |
| `join` | — | Switches to a text/voice channel by name in the current server |
| `nick` | — | **Stub** — points at Server menu → Change Nickname |
| `debugnow` | — | Hidden; opens the server debug window (not listed in the popup) |

Guild aliases (e.g. `s` → `shrug`) resolve through the same registry as built-in aliases.

## Extensibility

```rust
pub trait CommandRegistry {
    fn prefix(&self) -> &str;
    fn set_prefix(&mut self, prefix: String) -> Result<(), String>;
    fn register(&mut self, spec: CommandSpec) -> Result<(), String>;
    fn set_server_aliases(&mut self, aliases: Vec<(String, String)>) -> Result<(), String>;
    fn server_aliases(&self) -> &[(String, String)];
    fn resolve(&self, name: &str) -> Option<&CommandSpec>;
    fn suggestions(&self, partial: &str) -> Vec<&CommandSpec>;
}
```

Client module: `crates/banter/src/commands.rs`. Plugin packages may declare commands (see `commands` permission / roll sample); the desktop composer does not execute server plugin handlers yet — register client-side stubs via `CommandRegistry::register` when that lands.

## Code map

| Piece | Location |
| --- | --- |
| Client registry + parse | `crates/banter/src/commands.rs` |
| Composer popup / run | `crates/banter/src/ui.rs` (`draw_composer`, `try_run_chat_command`) |
| Settings modal | `Modal::ChatCommands` |
| Migration | `007_chat_commands.sql` |
| Store / HTTP | `Store::command_settings` / `set_command_settings`, `GET|PUT /v1/servers/{id}/commands` |
| Tests | `crates/banter/tests/commands.rs`, foundation `server_command_prefix_and_aliases_persist_per_guild` |
