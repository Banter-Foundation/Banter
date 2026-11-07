# Chat compose autosuggest

Local suggestion chips and ghost completions in the bottom message composer (BA-103). No cloud LLM.

## What you get

| Trigger | Source |
| --- | --- |
| `@` + prefix | Channel members (or DM peer) |
| `:` + prefix | Emote shortcodes **only if** the client supplies names (Emote Library / BA-83 hook; empty today) |
| Ordinary words | Bigram/trigram model trained with **own messages weighted highest**, then the current draft, then peers in the open channel, plus a small static phrase list |

**Accept:** Tab or click a chip. **Dismiss:** Escape (until the draft text changes).

Own-message learning also keeps a short session memory of your bodies from other channels/DMs you already opened (no extra fetch). Weights live in `suggest::{WEIGHT_OWN` / `WEIGHT_DRAFT` / `WEIGHT_PEER`.

## Code

- `crates/banter/src/suggest.rs` — token scan, engine, ranking, apply/dismiss
- `crates/banter/tests/suggest.rs` — predictor contract tests (includes own-vs-peer ranking)
- `crates/banter/src/ui.rs` `draw_composer` — chips, ghost text, keys; own-memory + draft ingest
- `crates/banter/src/theme.rs` — `SUGGEST_*` tokens

## Manual try

1. Send a few of your own messages with a distinctive phrasing, then type the opener — your next word should beat more-common peer phrasing.
2. Visit another channel in the same session; your earlier own lines should still influence suggestions.
3. Type `@` then a letter → mention chips; Tab inserts `@username `.
4. Escape hides suggestions; editing the draft brings them back. Enter still sends.
