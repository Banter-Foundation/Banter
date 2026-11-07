# Banter error codes

Banter surfaces failures with a **stable code**, a **short user message**, and optional **technical detail** for logs. Do not show opaque strings like `request failed`.

## Shape

| Field | Where | Purpose |
| --- | --- | --- |
| `code` | HTTP JSON `code`, toast suffix, logs | Stable `BANTER_*` token |
| `message` | HTTP JSON `error`, toast body | Short, plain language |
| `detail` | Client logs / `BanterIssue::log_text` | Method, path, status, transport text |

Toast / notice strip text is `{message} ({CODE})` via `format_banter_notice`.

Shared types live in `banter_server::errcode` (`BanterIssue`, `ERR_*` constants). Server `BanterError::code()` uses the same `BANTER_*` style.

## Voice / media (apply first)

| Code | User message (typical) | When |
| --- | --- | --- |
| `BANTER_VC_JOIN_FAILED` | couldn't join voice | `POST /v1/voice/join` |
| `BANTER_VC_LEAVE_FAILED` | couldn't leave voice | `POST /v1/voice/leave` |
| `BANTER_VC_FLAGS_FAILED` | couldn't update voice state | `POST /v1/voice/flags` (mute/deafen/camera/screen) |
| `BANTER_VC_PUBLISH_FAILED` | couldn't capture the screen | Go Live / screen capture open failures (BA-100; Windows DXGI `video_recorder` was raw `0x80070057`) |
| `BANTER_VC_DEVICE_FAILED` | Banter could not open this microphone/speakers/camera… | cpal / camera open failure |
| `BANTER_VC_LOOPBACK_FAILED` | couldn't capture audio from {label} | WASAPI speaker or per-app process loopback (BA-92; was raw `0x80070057`) |
| `BANTER_WS_CONNECT_FAILED` | couldn't connect realtime | `/v1/ws` connect |
| `BANTER_WS_DROPPED` | realtime connection dropped | `/v1/ws` read/write failure |
| `BANTER_HTTP_STATUS` | server returned HTTP {status} | Non-2xx with empty/non-catalog body |
| `BANTER_HTTP_TRANSPORT` | couldn't reach the Banter server | Timeout, connection refused, DNS |
| `BANTER_HTTP_PARSE` | couldn't read the server reply | Bad JSON / body |
| `BANTER_SERVER_TOO_OLD` | this Banter server is too old… | Host protocol missing or below client floor (BA-91) |
| `BANTER_CLIENT_TOO_OLD` | this Banter client is too old… | Client protocol below host `min_client_protocol` (BA-91) |
| `BANTER_VERSION_UNREACHABLE` | couldn't reach the Banter server to check its version | Health probe failed when a host was required (BA-91) |

Server domain codes (examples): `BANTER_NOT_VOICE_CHANNEL`, `BANTER_STORAGE`, `BANTER_INVALID_CREDENTIALS`. Full list: `BanterError::code()`. See also [version.md](version.md).

## Pattern for new call sites

1. Prefer wrapping the HTTP/WS result with a catalog code at the API method (`join_voice`, `set_flags`, …), not deep in the UI.
2. Keep the toast short; put method/path/status in `detail`.
3. Do not retry a failing voice-flags publish every frame — back off (`voice_flags_retry_ready`).
4. Migrate other screens as you touch them; do not boil the ocean in one change.

## BA-87 note

The red stage notice Owl saw as literal `request failed` came from the client collapsing an empty HTTP error body while **Voice Connected** (join already succeeded). The recurring caller is `POST /v1/voice/flags` from `finish_voice_flags`. That path now uses `BANTER_VC_FLAGS_FAILED` and includes HTTP status in the underlying detail.
