# Permanent platform branches

Owl (2026-11-08): these long-lived branches stay in the Banter repo. They are **not** feature branches and must not be deleted by cleanup jobs or agents.

| Branch | Role |
| --- | --- |
| `main` | Integration trunk |
| `linux` | Linux client / packaging line |
| `windows` | Windows client / packaging line |
| `mac` | macOS client / packaging line |
| `android` | Android client line (future native or wrapper work) |
| `website` | Banter website / web surface |

## Permanence rules (agents)

1. **Do not delete** `linux`, `windows`, `mac`, `android`, or `website`.
2. **Do not force-push** them unless Owl explicitly asks.
3. Feature work still uses short-lived `cursor/…` branches; merge to `main` first.
4. Temporary scratch and merged feature branches remain fair game for cleanup.

GitHub branch protection must be set by an admin in Settings → Branches (agents cannot set it). Until then, this document is the contract.

## iOS feasibility

Native iOS is **not feasible** on the current Rust + GLFW + OpenGL 3.3 + Dear ImGui desktop stack without a separate client. Options later: empty `ios` placeholder branch, native Swift client, or web shell over `website`. Do not create `ios` until Owl asks.
