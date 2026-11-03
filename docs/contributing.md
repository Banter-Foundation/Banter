# Contributing

Banter is a private repo ([SnowedInOwl/Banter](https://github.com/SnowedInOwl/Banter)). Keep docs useful for anyone who opens the tree after you.

## Before you change code

1. Read `.cursor/memory.md`, recent `.cursor/changelog.md` entries, and `.cursor/features.md`.
2. Prefer `D:\Projects\Project Banter` (or a worktree under `D:\Projects\Project Banter\worktrees\`, for example `…\worktrees\Banter-docs`). **Never** build or write Banter artifacts on `E:`.
3. If another agent already holds the primary checkout with uncommitted work, **do not fight them**. Use `git worktree add` under `worktrees/` or a read-only pass.
4. Ask Owl before inventing new features, dependencies, or requirements that are not already decided.

## Branches and worktrees

- Feature branches: short names that say the job (`ui-polish`, `settings-fixes`, `cursor/…`).
- Cloud agents append a stable suffix when required by their run instructions.
- Keep long-running experiments in separate worktrees with their own `target/` when possible so Cargo locks do not block peers.
- Do not force-push `main`. Do not move the `Version-1-test-1` release tag for unrelated UI commits.

## Commits and change notes

Every commit message should say **what** changed and **why**. Vague titles (“fixes”, “wip”) are not enough.

This repo uses **`.cursor/changelog.md`**. Append a new entry (never rewrite history of the file). Heading shape:

```markdown
## YYYY-MM-dd HH:mm:ss ±HH:MM [Username] — Short title
```

Put same-day entries next to each other in order. Start a new day below the previous day. Cover features, fixes, and verification (tests run, binary path, ports).

Also keep `.cursor/features.md` honest: foundation items marked `[*]` when present; deferred list stays until Owl removes it.

## Tests first for behavior

When adding API or settings behavior, write or extend the failing test **before** the implementation. Do not weaken `crates/banter-server/tests/foundation.rs` to greenwash a break. Client tests under `crates/banter/tests/` cover session, media helpers, settings rules, and theme metrics.

## Secrets and local data

Never commit:

- `%LOCALAPPDATA%\Banter\**` (or the Linux/macOS data dir)
- `auth.json`, `client-session.json`, `voice.json`, `stream.json`
- `.env`, secrets, PEMs

## Docs layout

| Path | Purpose |
| --- | --- |
| `README.md` | Quick start |
| `docs/overview.md` | Product + stack + paths + bind |
| `docs/build-and-run.md` | Windows/Linux/container |
| `docs/architecture.md` | Crates and modules |
| `docs/feature-status.md` | What works vs deferred |
| `docs/foundation.md` | Chosen foundation + deferred list |
| `docs/contributing.md` | This file |
| `docs/current-build.md` | Live observation notes |
| `.cursor/design.md` | Visual metrics for the temporary skin |

When you change how people run or understand Banter, update the matching doc in the same PR.
