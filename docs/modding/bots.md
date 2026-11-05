# Bots

A process you host that talks to Banter over HTTP and the websocket as its own account. Banter does not load the bot's code. Shared permission behavior is in [Modding](modding.md). Command shape matches [Plugins](plugins.md).

A bot is an account that a process you host uses through `/v1`. It shows up in the member list with a Bot badge. It has no password and no friend list. A user can open a DM with a bot that shares a server with them; friend checks are skipped for that pair.

## Account and token

Later schema: `users.bot` boolean, default false. `bot_tokens (token, user_id, created_at)`. `bots (user_id, owner_id, intents, interactions_url)`. Creating a bot does not insert a password hash that `POST /v1/login` will accept.

Owner routes, bearer is a normal user session:

- `POST /v1/bots` body `{ "username", "display_name", "intents" }` → `{ "token", "account" }`. Username and display name use the same rules as registration. `account` is the normal account object plus `"bot": true`. The token is returned only here and on rotate. `intents` may be omitted, which stores an empty list.
- `GET /v1/bots` → array of `{ account, intents, interactions_url }` for bots this user owns. No tokens.
- `GET /v1/bots/{id}` → one of those objects. `404` `not_found` when the caller does not own it.
- `PATCH /v1/bots/{id}` body, all optional: `{ "display_name", "intents", "interactions_url" }`. Replacing `intents` replaces the checklist. Intents removed from the list start returning `null` immediately.
- `DELETE /v1/bots/{id}` removes the bot user, tokens, guild installs, and its commands.
- `POST /v1/bots/{id}/token` → `{ "token" }` and deletes the previous token.

`intents` is the developer checklist. There is no default set. The names are `view_channels`, `read_messages`, `send_messages`, `manage_messages`, `connect_voice`, `presence`, `interactions`, `friends`, `servers`, `roles`, `plugins`, `debug`, `calls`, `board`, and `profile`. Unknown names return `400` `invalid`. `interactions_url` is `null` or an `https` URL at most 500 characters.

Bot-token routes:

- `GET /v1/me` → account with `bot: true`
- `PATCH /v1/me` body `{ "display_name" }`
- `POST /v1/me/avatar` and `DELETE /v1/me/avatar` — same image rules as users
- `GET /v1/bots/me/commands`
- `PUT /v1/bots/me/commands` replaces the full list. Body is an array of command objects, same shape as plugin commands, max 32. At most one replace per 10 seconds; further calls return `429` with `{ "error", "code": "rate_limited", "retry_after_ms" }`.

`POST /v1/login` and `POST /v1/register` do not create bots. `POST /v1/logout` on a bot token returns `400` `invalid`. Rotate the token to revoke it.

## Guild install

Adding a bot makes it a member. It does not create a second permission filter. `server_bots` stores `(server_id, bot_user_id)` and the intent list the admin was shown at that moment.

The bot appears in `GET /v1/servers/{id}/members` with `"bot": true` and no guild roles. It is not the owner.

Admin, owner or `manage_bots` (new permission bit):

- `GET /v1/servers/{id}/bots` → `{ bot, intents }` where `intents` is the developer checklist.
- `PUT /v1/servers/{id}/bots/{bot_id}` adds the bot. The client shows the checklist and asks the admin to allow it. The body is `{ "accept": true }`. The bot user must exist.
- `DELETE /v1/servers/{id}/bots/{bot_id}` removes membership and kicks the bot from that server’s voice channels.

After a developer changes intents, the next call that needs a newly checked intent returns `null` on that server until an admin `PUT`s the install again. Intents the developer removed return `null` without another admin step.

## HTTP the bot may call

Bearer is the bot token. The bot’s `intents` checklist is the list the developer checked. If the intent for a route is missing, the response is HTTP 200 and the body is JSON `null`. Nothing is changed. If the intent is present, the route runs and uses the normal errors (`not_member`, `not_found`, `not_author`, `rate_limited`, and the rest).

Intents and the routes they turn on:

- `view_channels` — `GET /v1/servers` (installed servers), `GET /v1/servers/{id}/layout`, `GET /v1/servers/{id}/members`, `GET /v1/users/{id}`
- `read_messages` — `GET /v1/channels/{id}/messages`, `GET /v1/dms`, `GET /v1/dms/{id}/messages`, plus message gateway events
- `send_messages` — `POST /v1/messages`, `POST /v1/dms`. Files stay unavailable while attachments are disabled, and the existing error still applies when this intent is checked.
- `manage_messages` — `DELETE /v1/messages/{id}` for any message in an installed server, `PATCH /v1/messages/{id}` for messages the bot authored
- `connect_voice` — `POST /v1/voice/join`, `POST /v1/voice/leave`, `POST /v1/voice/flags`, plus `voice` gateway events
- `presence` — `presence` gateway events
- `interactions` — `interaction` gateway events, `GET` and `PUT /v1/bots/me/commands`, and `POST /v1/interactions/{id}/reply`
- `friends` — friend request and friend list routes
- `servers` — `POST /v1/servers`, invites, nicknames
- `roles` — role create, edit, and member role assignment
- `plugins` — plugin install, enable, disable, delete
- `debug` — `GET /v1/debug`
- `calls` — DM call start, accept, decline, end
- `board` — board read, ops, and assets
- `profile` — profile patch, banner, and cosmetics routes beyond display name and avatar

`GET /v1/me`, `PATCH /v1/me`, and the avatar routes stay available with no extra intent. Join updates the voice member list. The websocket does not offer RTC offer/answer for bots in this version. Signaling frames the bot sends are ignored. A later doc will add media.

The admin who adds the bot sees this same checklist and allows it. The admin does not grant intents the developer left unchecked. Those keep returning `null`.

## Gateway

`GET /v1/ws` with the bot bearer. The first frame is:

```json
{
  "kind": "ready",
  "bot": true,
  "user": { "id": 4, "username": "chessbot", "display_name": "Chess", "has_avatar": false, "avatar_rev": 0, "bot": true },
  "guild_ids": [1],
  "intents": ["view_channels", "read_messages", "send_messages", "interactions"]
}
```

Later frames follow the checklist. An event whose intent is missing is not sent.

- `read_messages`: `message_created`, `message_edited`, `message_deleted` for channels the bot can see and for its DMs
- `presence`: `presence`
- `connect_voice`: `voice`, and `rtc_offer`, `rtc_answer`, `rtc_ice`, `subscribe_layer`, `wanted_layers`
- `interactions`: `interaction`
- `friends`: `friend_request`, `friends_changed`
- `calls`: `call`, `call_ended`
- `board`: `board_changed`

Client frames use the same rule. `activity` is always accepted. `rtc_*`, `subscribe_layer`, and `board_op` are accepted when `connect_voice` or `board` is checked, and ignored (no error frame) when it is not. How a bot sends audio and video is still a later revision. The frames themselves are not blocked once `connect_voice` is checked.

## Interactions

`PUT /v1/bots/me/commands` registers commands. Names must not collide with a plugin command on a server where both exist. The plugin command wins and the bot does not receive an interaction for that name; `POST /v1/commands` response includes `"handler": "plugin"`. When the bot handles it, `"handler": "bot"`.

Interaction event:

```json
{
  "kind": "interaction",
  "id": "8f1c...",
  "server_id": 1,
  "channel_id": 2,
  "user_id": 9,
  "command": "play",
  "options": { "opponent": 3 },
  "token": "single-use"
}
```

Reply, one time, within 15 minutes:

`POST /v1/interactions/{id}/reply`

Bearer may be the bot token or the interaction `token`. Body `{ "body": "<BFMD>", "reply_to": null }`. Creates a message authored by the bot in `channel_id`. Second reply returns `409` `conflict`. Expired token returns `404` `not_found`.

When `interactions_url` is set, the server also POSTs the same JSON, with header `Authorization: Bearer <interaction token>`, and waits up to 3 seconds. `200` body `{ "body": "<BFMD>" }` posts the message. Any other outcome leaves the gateway event in place so the bot can still `POST /v1/interactions/{id}/reply`.

`POST /v1/commands` from a member, when a bot owns the command and no plugin does:

- If the interactions URL returns a body in time, the HTTP response is that message and `"handler": "bot"`.
- Otherwise the HTTP response is `202` `{ "handler": "bot", "interaction_id": "..." }` and the member’s client shows the result when `message_created` arrives.

## Rate limits

Per bot token, HTTP `429` `rate_limited` with `retry_after_ms`:

- 50 requests in 10 seconds, all routes combined
- 5 `POST /v1/messages` in 5 seconds per channel
- 1 `PUT /v1/bots/me/commands` in 10 seconds

## Example

Create, install, connect, send:

```http
POST /v1/bots
Authorization: Bearer <user token>
{"username":"chessbot","display_name":"Chess","intents":["view_channels","read_messages","send_messages"]}

PUT /v1/servers/1/bots/4
Authorization: Bearer <user token>
{"accept":true}

GET /v1/ws
Authorization: Bearer <bot token>

POST /v1/messages
Authorization: Bearer <bot token>
{"target":{"type":"channel","id":2},"body":"Ready."}
```
