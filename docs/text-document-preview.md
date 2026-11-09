# Text document inline preview

**BA-121** (ask item 2). When a **text document** attachment is shown on a chat message, Banter draws a Discord-style expandable inline preview.

## Behavior

- **Who gets a preview:** attachments whose filename extension is treated as text (`.txt`, `.md`, `.json`, `.rs`, `.bfmd`, logs, configs, common source/text types). Media and binary extensions stay download-only.
- **Card:** filename, human-readable size, and **Save** (existing download path).
- **Body:** lazily fetched via `GET /v1/attachments/{id}`, decoded as UTF-8 (BOM stripped). Invalid UTF-8 or NUL bytes → “Preview unavailable”.
- **Expand / Collapse:** collapsed chrome soft-windows to `COLLAPSED_VIEW_LINES` (10). Expanded shows the **full** document. There is **no** line-count gate that refuses preview; a later pass may add a threshold.
- **Not in this change:** BFMD/code-block syntax highlighting, and the separate text-block message type (other workers). Composer upload stays disabled (BA-61); existing attachments still preview/download.

## Code

| Piece | Location |
| --- | --- |
| Detection, decode, expand helpers | `crates/banter/src/text_preview.rs` |
| Chat card + lazy fetch | `crates/banter/src/ui.rs` (`ensure_attachment_text`, message attachment rows) |
| Contract tests | `crates/banter/tests/text_preview.rs` |

Ask note: project store `docs/bfmd-code-preview-textblock-ask.md` item 2.
