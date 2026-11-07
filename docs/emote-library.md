# Emote Library (BA-83)

Vertical slice for Carbon-like emotes / stickers / GIFs plus Banter pack subscriptions.

## What shipped

- **Postgres:** `emote_packs`, `emote_pack_assets`, `emote_pack_subscriptions`, `server_emote_packs`, `message_media` (migration `007_emote_library`).
- **Upload:** emote ≤256KB; sticker ≤2MB; classic GIF ≤2MB / video GIF ≤4MB. GIF encodings share a **15s** hard duration cap (`classic_gif` one-loop Σ delays; `video` from MP4 `mvhd`). Error copy: `Max 15 seconds`.
- **Packs:** user or server-owned; mixed kinds OK; no seed/mock packs.
- **Subscribe:** personal subscribe unlocks pack in the Subscribed picker source; server packs auto-available to members (subscribe-to-keep for cross-server use).
- **Send:** emotes insert stable `:{e:id}:` into BFMD body; stickers are **Model A** (one sticker media, optional text); GIFs attach as message media.
- **BFMD:** `{e:N}` emote height in px (default 22, clamp 16–128), independent from text `{s}`.
- **Composer:** Emotes / Stickers / GIFs picker tabs (Server | Subscribed), pending media strip, send via `media_asset_ids`.

## HTTP

| Method | Path | Notes |
| --- | --- | --- |
| `POST` | `/v1/emote-packs` | Create pack |
| `POST` | `/v1/emote-packs/{id}/assets?name&kind&content_type` | Raw body upload |
| `POST` / `DELETE` | `/v1/emote-packs/{id}/subscribe` | Personal subscribe |
| `GET` | `/v1/emote-assets/{id}` | Asset bytes |
| `GET` | `/v1/emote-picker?kind&source&server_id` | Picker list |
| `POST` | `/v1/messages` | Accepts `media_asset_ids` |

Files live under `BANTER_DATA/emotes/{asset_id}`.

## Deferred / follow-ups

- Full Emote Library settings page (create/upload UI beyond API)
- Inline texture decode of emotes/stickers/GIFs in chat (v1 shows shortcode / media labels)
- Video GIF muted autoplay-in-view / picker hover playback chrome
- Favorites, discovery, shortcode autocomplete on `:`
- Unicode emoji keyboard; reactions; Tenor/Giphy
- Classic GIF auto-transcode to video

Authoritative product plan: project store `docs/emote-library.md`.
