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

## Open Questions & Tasks.

use the github project issue tracker to track open questions and state of issues and tasks.

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
