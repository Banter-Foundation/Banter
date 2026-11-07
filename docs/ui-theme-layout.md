# UI theme and layout branch

**Branch:** `cursor/ui-theme-layout-cf5e`  
**Worktree (Black-Birb):** `D:\Projects\Banter-ui-theme`  
**Purpose:** Dedicated line of work for theme tokens and signed-in shell layout. Do not merge to `main` unless Owl asks.

The Discord-like skin stays temporary and workable. This branch does not redesign the product look in one pass. It adds clear boundaries so later agents can change colors, widths, and regions without fighting the Dev Settings checkout at `D:\Projects\Banter`.

## Module boundaries

| Module | Owns | Does not own |
| --- | --- | --- |
| `crates/banter/src/theme.rs` | Colors, sizes, radii, font sizes, small geometry helpers (rail pill, chat pad, tooltip rect, …) | Screen flow, HTTP, media |
| `crates/banter/src/layout.rs` | Named `Region`s, `ShellColumns`, `shell_columns` / `discord_shell_columns` | Drawing, theme token values |
| `crates/banter/src/skin.rs` | Shared Dear ImGui draw helpers (text, buttons, inputs, menus, tooltips) | Layout math, business rules |
| `crates/banter/src/ui.rs` | Screens that compose regions (sign-in, shell, settings chrome, chat, voice) | New hard-coded column widths or one-off colors |
| `crates/banter/src/settings.rs` | Settings rules / `SettingsState` (drawable-free for unit tests) | Theme tokens |

Rules for later agents:

1. New color or pixel size → add a constant in `theme.rs`, then use it.
2. New or moved shell area → name it in `layout::Region` (or document why a local rect is enough), compute geometry in `layout` when it is shared.
3. Reusable control chrome → `skin.rs`.
4. Keep the temporary Discord skin compiling and looking familiar until Owl retires it.

## Layout regions

Top: **title-bar** (28, client-drawn Mac-style caption). Below that, left to right: **rail** (72) → **sidebar** (240) → **stage** (flex) → **members** (240, optional).

Also named: **header** (48, over stage + members), **voice-panel** and **user-panel** (bottom of sidebar), **composer** (bottom of stage in text channels).

`layout::shell_columns(width, show_members)` reads widths from `theme::{RAIL_W,SIDEBAR_W,MEMBERS_W}`. Prefer `shell_columns` in new code; `discord_shell_columns` remains as an alias. Column rects start at `TITLE_BAR_H` so the shell sits under the caption.

Visual metrics detail: `.cursor/design.md`.

## Tests

- `crates/banter/tests/layout.rs` — region names, theme-backed columns, rect helpers
- `crates/banter/tests/theme.rs` — token and shell metrics
- `crates/banter/tests/titlebar.rs` — caption geometry, traffic lights, drag / double-click

Run from this worktree with its own `target/` (set `CARGO_TARGET_DIR` if another Banter exe holds a lock):

```
cd D:\Projects\Banter-ui-theme
cargo test -p banter --test layout --test theme --test titlebar
```

## Coordination

- Primary dirty checkout for Dev Settings: `D:\Projects\Banter` on `cursor/dev-server-settings-0674` — do not fight it.
- Older UI polish worktree: `D:\Projects\Banter-ui` on `ui-polish`.
- Never build or write Banter artifacts on `E:`.
