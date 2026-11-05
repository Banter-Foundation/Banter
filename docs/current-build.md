# Current build notes

Observed on **Black-Birb** while documenting, **2026-11-03** (America/Chicago), with a Postgres refresh **2026-11-04** after the cutover punch list. Docs work originally used worktree `D:\Projects\Banter-docs` on branch `cursor/banter-build-docs-1fda` so the dirty primary checkout was left alone.

**Path remap (2026-11-03 later):** primary checkout is now `D:\Projects\Project Banter`; that docs worktree is `D:\Projects\Project Banter\worktrees\Banter-docs`. Paths below in the observation table are historical from the docs pass.

Base revision originally documented: **`aa61044`** (`origin/main` — “Run Banter natively on Linux with XDG paths and PipeWire-ready capture.”). Runtime storage since the Postgres cutover (`46917ad` and follow-ups) is described under **Data layout**.

## Data layout (Postgres era)

| Kind | Where |
| --- | --- |
| Accounts, servers, channels, messages, profile rows | PostgreSQL via `BANTER_DATABASE_URL` (default `postgres://banter:banter@127.0.0.1:5432/banter`) |
| Compose DB volume | `banter-pg` (host port `BANTER_PG_PORT`, default 5432) |
| File bytes (avatars, banners, attachments, board assets) | `BANTER_DATA` or `%LOCALAPPDATA%\Banter` / `$XDG_DATA_HOME/Banter` — under `avatars/`, `banners/`, `files/` |
| Client prefs | Same data dir: `client-session.json`, `voice.json`, `stream.json`, `home.json`, `dev-server.json`, `banter-dock.ini` |

There is **no** live `banter.db`. Leftover SQLite files under the data dir (if any) are ignored; Banter does not import them. Schema boot runs `schema.sql` then versioned migrations so existing Postgres volumes gain new columns/tables without a wipe. See [build-and-run.md](build-and-run.md) for the local port conflict (`BANTER_PG_PORT` + matching `BANTER_DATABASE_URL`).

## Process already running (2026-11-03 observation)

Did **not** start a second client (other agents and an open window already owned the desktop).

| Check | Result |
| --- | --- |
| Process | `banter` pid **34512**, title **Banter**, responding |
| Binary | `D:\Projects\Banter\target\debug\banter.exe` (started ~11:04 AM; path pre-remap) |
| Window | ~1296×835 at screen position (78, 92) |
| Health | `GET http://127.0.0.1:3970/v1/health` → `ok` plus `version` / `protocol` / `min_client_protocol` (BA-91; older hosts may still be `{"ok":true}` only) |
| Data dir | `%LOCALAPPDATA%\Banter` — session/prefs + file bytes; SQL store is Postgres (see Data layout) |

## What the UI showed

`PrintWindow` (flag `PW_RENDERFULLCONTENT`) captured a usable frame of the OpenGL client. Plain `CopyFromScreen` / GDI sampling returned near-uniform dark pixels and is **not** reliable for this window.

From the PrintWindow capture (`media/banter-build-docs-printwindow.png` in the Project store):

- Discord-like dark shell: server rail, channel sidebar, chat stage, member list
- Server **Cool Test Server** selected (rail mark **CT** / similar), Home **B**, green **+**
- Text channel **# general** selected; voice channel **General** listed
- One chat message from **Owl**: “Hi” with timestamp style “Today at …”
- Composer placeholder **Message #general** with attachment **+**
- User bar: display name **Owl**, mute / deafen / settings icons, avatar present
- Member list showed Owl with owner crown; presence grouping visible

Earlier same-day Project store shots (for comparison, not re-clicked in this pass): login/register, friends home, invite/create channel/nickname modals, voice connected stage, chat hover toolbar — under the coordinating agent’s `media/ui-*.png`.

## What was not exercised here

To avoid colliding with Camera/voice-bar and Voice stream hand-check agents:

- No voice join / disconnect
- No camera or Go Live
- No Settings navigation
- No second-client register/send
- No container rebuild

Those paths are covered by prior changelog entries and the other agents’ tasks.

## How to reproduce a similar look

```powershell
cd "D:\Projects\Project Banter"   # or a clean worktree under worktrees\
# Developer shell so cl.exe is available
# Ensure Postgres is up (see build-and-run.md Local Postgres port if 5432 is taken)
cargo run -p banter
# optional: Invoke-RestMethod http://127.0.0.1:3970/v1/health
```

For a cold login UI without disturbing a shared session file, set `BANTER_DATA` to an empty directory for that process only.
