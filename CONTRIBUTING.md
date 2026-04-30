# Contributing

## Folder convention

Every component in this repo follows the same structure:

```
component/
  README.md          ← The aspirational spec — what this component should be
  parking_lot.md     ← Open questions that need human input
  changelog.md       ← History of changes to this component
  src/
    README.md        ← What is actually implemented today (honest, specific)
    sprint.md        ← Current work and next steps to close the gap between src/README and README
    *.rs             ← Source code
```

**README.md** describes the end goal. **src/README.md** describes where we actually are — what works, what's stubbed, what's missing. **src/sprint.md** is the midway point — current work and next steps. **parking_lot.md** captures open design questions. **changelog.md** records what changed and why.

When picking up work on a component, read all three: the spec (README), the status (src/README), and the plan (src/sprint.md).

---

## Changelog

Every change to any document in this repo should be logged into the `changelog.md` for that folder **and** propagated as a one-liner up to every parent folder's `changelog.md`, all the way to root. See [CLAUDE.md](CLAUDE.md) for the full propagation rules.

Changelog entries are append-only, latest entry on top. Each entry is a level-3 heading followed by a short prose body combining what changed and why, separated by blank lines.

Heading format: `### {Author} · {Timestamp}` — e.g. `### broodsugar's claude · Apr 2, 21:03 HKT, 2026`.
Body: what changed and why, in prose. Keep it tight — one short paragraph is usually enough.

Humans may manually add to the changelog using their name as the author.

Because the changelogs are updated so frequently, it is best practice to do a git pull right before updating the changelog, and push your latest changes right after updating the changelog. This will help avoid conflicts and save time.

---

## Parking lot

Open questions go in the `parking_lot.md` in the relevant folder. Cross-cutting questions go in the root `parking_lot.md`. Parent folders maintain a structured summary of their subfolders' parking lots.

To answer a parking lot item, write your answer inline beneath it. An agent will then replace it with a "Propagate" task to update the docs when you're ready.
