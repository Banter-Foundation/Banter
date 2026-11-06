# Version system and checker (BA-91)

Banter releases use labels like `Version-1-test-N`. Compatibility is **not** decided by equal strings. Clients and servers advertise a numeric **protocol** revision so a newer client can refuse a stale host (the live case that motivated this: a pre–PR #19 server still answering `{"ok":true}` while guild VC media was broken).

## Constants (`banter-server` `version.rs`)

| Constant | Meaning |
| --- | --- |
| `BANTER_VERSION` | Human label for this build (bump when cutting a release tag) |
| `PROTOCOL` | Wire / feature revision for this build |
| `MIN_SERVER_PROTOCOL` | Oldest server protocol this client accepts |
| `MIN_CLIENT_PROTOCOL` | Oldest client protocol this server accepts |

Bump `PROTOCOL` (and floors as needed) when the client requires server behavior older hosts lack.

## API

`GET /v1/health` and `GET /v1/version` return the same JSON shape:

```json
{
  "ok": true,
  "version": "Version-1-test-13",
  "protocol": 1,
  "min_client_protocol": 1
}
```

Older hosts that only return `{"ok":true}` are treated as protocol `0` and fail the client’s `MIN_SERVER_PROTOCOL` check.

## Client checker

On startup (when a session is restored), before sign-in/register, and when Developer Local/Remote reconnects, the client:

1. Calls `GET /v1/health`.
2. Runs `check_remote_version`.
3. **Blocks** with a clear status / settings error and a `BANTER_*` code when incompatible, and drops any restored session.
4. **Warns** (Developer settings) when protocols match but release labels differ.

| Code | When |
| --- | --- |
| `BANTER_SERVER_TOO_OLD` | Host protocol missing or below `MIN_SERVER_PROTOCOL` |
| `BANTER_CLIENT_TOO_OLD` | Local `PROTOCOL` below the host’s `min_client_protocol` |
| `BANTER_VERSION_UNREACHABLE` | Health probe failed while a reachable host was required |

Notice text follows the BA-88 pattern `{message} ({CODE})`. Codes live in `banter_server::errcode` with the shared catalog (see [errors.md](errors.md)).

Client and server versions also show under **User Settings → Developer**.
