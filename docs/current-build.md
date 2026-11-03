# Current build notes

Observed on **Black-Birb** while documenting, **2026-11-03** (America/Chicago). Docs work used worktree `D:\Projects\Banter-docs` on branch `cursor/banter-build-docs-1fda` so the dirty primary checkout (`D:\Projects\Banter` on Camera/voice-bar work) was left alone.

Base revision documented: **`aa61044`** (`origin/main` — “Run Banter natively on Linux with XDG paths and PipeWire-ready capture.”).

## Process already running

Did **not** start a second client (other agents and an open window already owned the desktop).

| Check | Result |
| --- | --- |
| Process | `banter` pid **34512**, title **Banter**, responding |
| Binary | `D:\Projects\Banter\target\debug\banter.exe` (started ~11:04 AM) |
| Window | ~1296×835 at screen position (78, 92) |
| Health | `GET http://127.0.0.1:3970/v1/health` → `{"ok":true}` |
| Data dir | `%LOCALAPPDATA%\Banter` present (`banter.db`, WAL, `avatars/`, `files/`, `client-session.json`, `voice.json`, `stream.json`, `banter-dock.ini`) |

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
cd D:\Projects\Banter   # or a clean worktree on main
# Developer shell so cl.exe is available
cargo run -p banter
# optional: Invoke-RestMethod http://127.0.0.1:3970/v1/health
```

For a cold login UI without disturbing a shared session file, set `BANTER_DATA` to an empty directory for that process only.
