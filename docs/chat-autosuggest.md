# Chat compose autosuggest

Local suggestion chips and ghost completions in the bottom message composer (BA-103). No cloud LLM.

## What you get

| Trigger | Source |
| --- | --- |
| `@` + prefix | Channel members (or DM peer) |
| `:` + prefix | Emote shortcodes **only if** the client supplies names (Emote Library / BA-83 hook; empty today) |
| Ordinary words | Bigram/trigram model over recent message bodies + a small static phrase list |

**Accept:** Tab or click a chip. **Dismiss:** Escape (until the draft text changes).

## Code

- `crates/banter/src/suggest.rs` — token scan, engine, ranking, apply/dismiss
- `crates/banter/tests/suggest.rs` — predictor contract tests
- `crates/banter/src/ui.rs` `draw_composer` — chips, ghost text, keys
- `crates/banter/src/theme.rs` — `SUGGEST_*` tokens

## Manual try

1. Open a text channel with members and some history.
2. Type `@` then a letter → mention chips; Tab inserts `@username `.
3. Type `thank` or `sounds ` → phrase ghost/chips from static list / history.
4. Escape hides suggestions; editing the draft brings them back.
5. Enter still sends as before.
