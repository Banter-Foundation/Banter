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

Created from `main` at `3f80f2b7e4d6021e90f9d7c1cf98ce3b4eca974e` (2026-11-08).

## Permanence rules (agents)

1. **Do not delete** `linux`, `windows`, `mac`, `android`, or `website`.
2. **Do not force-push** them unless Owl explicitly asks.
3. Feature work still uses short-lived `cursor/…` (or similar) branches; merge to `main` first, then bring platform lines forward when Owl asks.
4. Temporary scratch and merged feature branches remain fair game for cleanup (see Contributing).

### GitHub branch protection

The GitHub tools available to agents cannot set branch protection rules. Owl (or an org admin) should turn on protection for these refs in **Settings → Branches** (prevent deletion; optionally require PRs). Until that UI step is done, this document is the contract agents follow.

## iOS feasibility

**Native iOS Banter is not feasible on the current stack without a separate client.**

Today's desktop client is **Rust + GLFW + OpenGL 3.3 + Dear ImGui (docking)**. That stack does not ship as a normal App Store iOS app:

- GLFW / desktop OpenGL 3.3 are not the iOS UIKit / Metal path.
- Dear ImGui can run on Metal in specialized ports, but Banter's windowing, input, and OS chrome assume a desktop GLFW host.
- App Store rules, background voice, and push notifications need Apple frameworks Banter does not use yet.

**Practical options later (not chosen yet):**

1. Keep an optional permanent `ios` branch as a placeholder (same pattern as `android`) once Owl wants the empty line reserved.
2. Ship iOS later as a **different client** (SwiftUI / native, or a web-first shell over `website`) that talks to the same Banter server APIs.
3. Do **not** expect the current desktop binary to cross-compile into a store iOS build.

Until Owl picks one of those, **do not create an `ios` branch** and do not start an iOS client in this repo.
