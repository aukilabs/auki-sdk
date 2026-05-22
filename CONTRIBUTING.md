# Contributing

## Folder convention

This repo follows a consistent folder structure for this project.

```
VISION.md      ← The aspirational spec — what this project should be.
README.md      ← Current state of the repo.
crates/*
  VISION.md      ← The aspirational spec — what this component should be
  README.md      ← Current state of the component.  
  src/
    *.rs             ← Source code
```

**VISION.md** describes the end goal. **README.md** describes where we actually are — what works, what's stubbed, what's missing.

All Project managment is done through the associated github project, boards and issues.

---

## READMEs

Each crate has a `README.md` describing the current state of the crate and how and when to use it. When you add a new crate or change what an existing crate does, update its `README.md` first, then propagate up to keep [`crates/README.md`](README.md) — the quick-overview index — accurate.

---
## Commit messages:

Commit messages should be clear, concise, and follow the [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/) specification.
the body should follow the convention of answering the following questions:
- why - what problem are we solving.
- how - how are we approaching this.
- what - what did we acctually do.

## Commit rules:
before making a commit, ask the user for final clarification on the why.

---

## GitHub Project Board

All work — questions, research, tasks, in-flight implementation — is tracked on the [SDK Kanban](https://github.com/orgs/Aukilabs/projects/5). Every commit should be traceable to a card on this board.

### Columns

| Column | Meaning |
|---|---|
| **Questions** | Blocked — needs a human decision before it can move. |
| **Exploring** | Active research / spike work in progress. |
| **Tasks** | Scoped and ready to be picked up. Has enough acceptance criteria for someone to implement. |
| **In progress** | Someone is actively implementing this on a branch. |
| **In review** | A PR is open and linked to the card. |
| **Done** | Merged and closed. |

### Typical flow

```
  Questions ──► Exploring ──► Tasks ──► In progress ──► In review ──► Done
       │            │            │
       │            └────────────┴──► Done (no implementation needed)
       │
       └──► Done (won't-do)
```

- **Questions exit when assigned.** The assignee decides where it goes: Exploring (will research), Tasks (now scoped), In progress (just do it now), or Done (won't-do).
- **Exploring exits when the research wraps up.** Destinations: Tasks (now scoped), Done (research-only, conclusion captured in the card), or Questions (surfaced a blocker that needs a decision).
- **Tasks exit when someone picks the card up to implement it.**
- **In progress → In review** when the PR opens. Use `Closes #N` in the PR body so GitHub auto-links it.
- **In review → Done** happens automatically on merge.

Rules for AI agents touching the board are in [`CLAUDE.md`](CLAUDE.md#project-board).

---

## Git hygiene

`develop` is the integration branch. All work happens on feature branches off `develop`.

### Branching

- **Branch off `develop`** for any new work. Never commit to `develop` directly.
- **Naming:** `type/issue-N-slug` where `type` is the Conventional Commits type (`feat` / `fix` / `chore` / `docs` / `refactor` / `test`), `N` is the SDK Kanban card's GitHub issue number, and `slug` is short kebab-case. Example: `chore/180-refresh-dataproducts`.
- **One branch per card.** If you're starting work on a card, you should be on its branch — not someone else's.
- **Wrong-branch detection.** Before starting any work, check `git status` and the current branch. If the work doesn't belong on the current branch, switch: stash if dirty, then `git checkout develop && git pull --ff-only` and start a fresh branch.

### Staying current with develop

Run `git checkout develop && git pull --ff-only` at these moments:

- **After your PR is merged.** Then delete the local and remote feature branch.
- **Before creating a new branch** for any new work.
- **Before opening a PR**, so you can rebase your branch onto the latest develop.

On a long-running branch, pull develop and **rebase** your branch onto it whenever you sit down to work (or at least daily). Prefer rebase over merge to keep history linear.

### Worktrees

Use `git worktree` when the current branch's working tree must stay untouched:

- Reviewing a PR locally while your in-flight work stays intact.
- Parallel work in a second worktree.
- Comparing builds across branches.

For serial work ("save this and switch tasks"), `git stash` + `git checkout` is enough. A worktree is overkill.

### Commits & push

- Commit at logical milestones, not WIP noise. Each commit should ideally compile and pass tests.
- Follow [Commit messages](#commit-messages) — Conventional Commits with `why / how / what` in the body.
- Push when ready to share or open a PR. `--force-with-lease` is OK on your own feature branch. **Never** force-push to `develop`.
- After PR merge, delete the local and remote feature branch.

---

## Tagging

Tags mark **consumer-coordination points**, not PR merges. A new tag is appropriate when:

- A downstream consumer (boosterapp, Park, etc.) is ready to bump and needs a pinnable reference.
- A coherent set of changes is sealed and ready for distribution.

**Don't cut a tag for every merged PR.** `develop` is the integration branch — let it accumulate multiple PRs between tags so consumers do one bump per coordination round-trip rather than chasing each merge. The cost of an extra commit on develop is zero; the cost of a bumped tag is real (every consumer eventually pulls it).

When in doubt, hold off. A consumer asking "can we tag now?" is the strongest signal a tag is warranted.

Tag style: **annotated**, message format `vX.Y.Z — <one-line description>` to match the existing history (`v0.0.2`..`v0.0.5`).


## Glossary
Use the glossary for definitions of all key terms specific for this project and repo.
Update the glossary as terms are used and add new terms as they are encountered, always ask the user before adding or removing terms.
