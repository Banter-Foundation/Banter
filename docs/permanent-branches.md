# Permanent platform branches

Owl (2026-11-08): these long-lived branches stay in the Banter repo. They are **not** feature branches and must not be deleted by cleanup jobs or agents.

## Branch descriptions

GitHub does not expose a remote “branch description” field via API. These texts are the canonical descriptions (also set locally with `git config branch.<name>.description` on Black-Birb).

| Branch | Description |
| --- | --- |
| `main` | Universal shared trunk of Banter — the base all permanent platform-specific branches (linux, windows, mac, android, website) build from and keep in sync with. |
| `linux` | Permanent Linux platform line — Linux client builds, packaging, and Linux-specific Banter work. |
| `windows` | Permanent Windows platform line — Windows client builds, packaging, and Windows-specific Banter work. |
| `mac` | Permanent macOS platform line — Mac client builds, packaging, and macOS-specific Banter work. |
| `android` | Permanent Android platform line — Android client builds, packaging, or wrapper work for Banter on Android. |
| `website` | Permanent website line — Banter website / web surface builds and content. |

Created from `main` at `3f80f2b7e4d6021e90f9d7c1cf98ce3b4eca974e` (2026-11-08).

## Permanence rules (agents)

1. **Do not delete** `linux`, `windows`, `mac`, `android`, or `website`.
2. **Do not force-push** them unless Owl explicitly asks.
3. Feature work still uses short-lived `cursor/…` (or similar) branches; merge to `main` first, then bring platform lines forward when Owl asks.
4. Temporary scratch and merged feature branches remain fair game for cleanup (see Contributing).

### GitHub branch protection

Owl (or an org admin) should turn on protection for these refs in **Settings → Branches** (prevent deletion; optionally require PRs). Until that UI step is done, this document is the contract agents follow.

## iOS feasibility

**Native iOS Banter is not feasible on the current stack without a separate client.** See also [ios-distribution-without-app-store.md](../../../../bc-e4b22778-c5d8-48db-9fd0-9e26860dabb5/docs/ios-distribution-without-app-store.md) in the Project store for AltStore / no–App Store options.

Until Owl picks a path, **do not create an `ios` branch** and do not start an iOS client in this repo.
