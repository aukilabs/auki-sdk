# Agents Guide — Auki SDK

This file is for AI agents reading this repo.

## What this repo is

This is the design foundation for the Auki SDK — a human and AI readable wiki and task repository that defines how the network works, what it is made of, and what needs to be built.

## Root files


- `VISION.md` — The aspirational spec — what this project should be
- `README.md` — start here. Overview of repo and crates.
- `CONTRIBUTING.md` — folder convention, project management and git workflows.
- `CLAUDE.md` — this file. Rules for AI agents.
- `GLOSSARY.md` — definitions of all key terms.


## Rules

### When in doubt, surface it

Do not resolve ambiguities unilaterally. Flag them to the developer and propose filing a Question on the SDK Kanban project (see below).

### Project board

All work is tracked on the [SDK Kanban](https://github.com/orgs/Aukilabs/projects/5). Columns and meanings are documented in [`CONTRIBUTING.md`](CONTRIBUTING.md#github-project-board). Agents follow these rules:

1. **Card creation always asks first.** Do not open cards autonomously — not even Questions. When you would otherwise file something (ambiguity, discovered bug, follow-up cleanup, a spike worth tracking), propose it to the developer: *type* (Question / Task / Exploring), *title*, one-paragraph body, *target column*. The developer creates it, or tells you to.

2. **You may move a card you are actively working on:**
   - `Exploring → Tasks` when the research scoped the work into a concrete task.
   - `Exploring → Done` when it was research-only and the conclusion is captured in the card.
   - `Exploring → Questions` if the research surfaced a blocker that needs a human decision.
   - `Tasks → In progress` when starting work. Assign yourself (or the developer), then create the branch off `develop`.
   - `In progress → In review` immediately after opening the PR — **move it yourself; do not rely on automation.** Put `Closes #N` in the PR body so GitHub auto-links the PR to the issue (and auto-closes it on merge).
   - `In review → Done` — verify after merge. If automation didn't fire, move it manually.

3. **Questions move when assigned.** An unassigned Question is awaiting a decision — don't touch it. Once a Question is assigned (to you, the developer, or anyone), the assignee owns it and may move it onward: `Questions → Exploring` (will research), `Questions → Tasks` (now scoped), `Questions → In progress` (small enough to just do), or `Questions → Done` (won't-do). Agents do not self-assign — wait for the developer to assign it.

4. **Do not touch cards you do not own.** Column, assignees, Priority, Size — none of it. If you think a card should move (e.g., a Question is unblocked by something you discovered), say so in chat and let the developer act.

5. **Never set Priority or Size** without explicit instruction from the developer, even on your own cards.

### Git hygiene

Full reference in [`CONTRIBUTING.md`](CONTRIBUTING.md#git-hygiene). Agent-specific rules:

1. **Check the branch before starting work.** Run `git status` and confirm the current branch matches the card you're about to touch. If the requested work doesn't belong on the current branch, stop and switch: stash if dirty, then `git checkout develop && git pull --ff-only` and create a fresh branch.

2. **Branch off `develop` only.** Naming: `type/issue-N-slug` (e.g. `chore/180-refresh-dataproducts`). One branch per card. Never commit to `develop` directly.

3. **Checkout `develop` and `git pull --ff-only` at these moments:**
   - **After your PR is merged** — then delete the local and remote feature branch (autonomous, no need to ask).
   - **Before creating a new branch.**
   - **Before opening a PR**, so you can rebase your branch onto the latest develop.

4. **Long-running branches: rebase onto develop** whenever you sit down to work, and again before opening the PR. Prefer rebase over merge to keep history linear.

5. **Use a worktree only when the current branch's tree must stay untouched** (reviewing a PR locally, parallel work the developer asked for, comparing builds). For "save this and switch tasks", plain `git stash` + `git checkout` is enough.

6. **Push policy.** Push when ready to share or open the PR. `--force-with-lease` is OK on your own feature branch. **Never** force-push to `develop`.
