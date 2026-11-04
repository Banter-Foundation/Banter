# Shared integration branch: `test/owl-fixes`

Sibling agents land Owl-fix fixes here first — **not** on `main`.

Branch: [`test/owl-fixes`](https://github.com/SnowedInOwl/Banter/tree/test/owl-fixes)  
Base: current `origin/main` at creation (`89f41b0`). Worktree: `D:\Projects\Project Banter\worktrees\Banter-owl-fixes`.

## Merge protocol

1. Do your work on a **short-lived feature branch** (name the job; keep commits clear).
2. When done, **rebase onto `test/owl-fixes`** (`git fetch origin` then `git rebase origin/test/owl-fixes`).
3. **Merge into `test/owl-fixes`** (PR or merge). Do **not** merge into `main` from these feature branches.
4. Commit messages must say **what** changed and **why** — no vague “wip” / “fixes”.
5. Stay under `D:\Projects\Project Banter` (or a worktree under `worktrees\`). **Never touch `E:`.**

`main` stays for reviewed, coordinated promotion only — not day-to-day Owl-fix landings.
