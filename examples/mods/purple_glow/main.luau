-- Dark purple / neon shell inspired by a Discord-style night theme.
-- Uses banter.theme.shell + banter.theme.set per docs/modding/mods.md.
-- Only the `ui` permission is required; disable clears the patch.

local shell = {
  rail_bg = 0x120B1A,
  sidebar_bg = 0x1C0A2E,
  stage_bg = 0x160821,
  panel_bg = 0x180C28,
  composer_bg = 0x241536,
  input_bg = 0x1E142C,
  text = 0xE8E6ED,
  muted = 0x8E8499,
  accent = 0xB24BFF,
  danger = 0xF23F43,
  row_hover = 0x2A1A40,
  row_selected = 0x3A2A56,
}

local widgets = {
  text = 0xE8E6ED,
  muted = 0x8E8499,
  heading = 0xF5F2FA,
  danger = 0xF23F43,
  link = 0xC77DFF,
  accent = 0xB24BFF,
  background = 0x160821,
  input_bg = 0x1E142C,
  button = 0x7A2FD4,
  button_hover = 0x9346EB,
  button_text = 0xFFFFFF,
}

banter.render(function()
  banter.theme.shell(shell)
  banter.theme.set(widgets)
end)
