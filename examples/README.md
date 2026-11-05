# Sample mods and plugins

Mods are folders. Copy one into the Banter data directory under `mods/`, then enable it in Settings → Mods.

| Folder | What it does |
| --- | --- |
| `mods/ping` | Header count and a Ping button. The count stays while the mod is enabled. |
| `mods/status_chip` | Header label `Local`. |
| `mods/warm_shell` | Warmer rail, sidebar, and stage. Later mods in the load order can replace those colors. |
| `mods/purple_glow` | Dark purple neon shell (rail / sidebar / stage / rows / accents). Enable in Settings → Mods. |

Plugins are folders or zip files in the Banter data directory under `plugins/`, next to `mods/`. Right-click a server and choose **Add Plugin** to install one on that server. A zip can also be posted to `POST /v1/servers/{id}/plugins`.

| Folder | Client | What it does |
| --- | --- | --- |
| `plugins/welcome` | none | After you allow it, every new channel message on that server is changed to `test`. |
| `plugins/roll` | none | Command `roll` posts “Rolled N” in that channel. |
| `plugins/notes` | optional | Stores a note body and adds the channel kind `notes.note`. The client half draws a Notes label after the member accepts it. |
