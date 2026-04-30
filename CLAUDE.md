# Agents Guide — Auki SDK

This file is for AI agents reading this repo.

## What this repo is

This is the design foundation for the Auki SDK — a human and AI readable wiki and task repository that defines how the network works, what it is made of, and what needs to be built.

## Core concept

The SDK has two core operations: `convert_time` (agree on time) and `convert_pose` (communicate across coordinate systems). Everything else — clusters, maps, detectors, credits — is infrastructure that produces the transforms these two operations consume. When reading or writing any document in this repo, keep this framing in mind.

## Folder convention

Every component follows the same structure:

```
component/
  README.md          ← The aspirational spec
  parking_lot.md     ← Open questions
  changelog.md       ← Change history
  src/
    README.md        ← What is actually implemented today
    sprint.md        ← Current work and next steps
    *.rs             ← Source code
```

When reading a component, check all three layers: spec (README), status (src/README), plan (src/sprint.md).

## Root files

- `README.md` — start here. Overview of the network, repo structure, and component index.
- `CONTRIBUTING.md` — folder convention, changelog format, parking lot workflow.
- `CLAUDE.md` — this file. Rules for AI agents.
- `Glossary.md` — definitions of all key terms.
- `parking_lot.md` — root-level open questions and cross-cutting design decisions.
- `changelog.md` — the global timeline of all changes across the repo.

## Rules

### Hierarchical propagation

Both `parking_lot.md` and `changelog.md` exist at every level of the folder hierarchy. They follow the same propagation pattern:

1. **Start at the leaf.** Make your change in the `parking_lot.md` or `changelog.md` in the same folder as the file you changed.
2. **Propagate upward.** Update the parent folder's corresponding file — a structured summary for parking lots, a one-liner for changelogs.
3. **Repeat until root.** Continue propagating up through every intermediate folder until you reach the root-level file.

Do this immediately after every change. Do not batch.

### parking_lot.md

- **Leaf parking lots** contain the full detail of each open question.
- **Parent parking lots** are structured summaries — they link to each subfolder's parking lot with item counts and one-line descriptions.
- Append questions to the parking lot in the folder where the question belongs. If the question is cross-cutting, use the root-level `parking_lot.md`.

### changelog.md

- **Append-only.** Never remove or edit existing entries.
- **Leaf changelogs** contain the detailed entry. See `CONTRIBUTING.md` for the format.
- **Parent changelogs** contain a one-liner summary of the change.
- The root `changelog.md` is the global timeline — every change in the repo, no matter how deep, should appear there as a one-liner.

### Resolving parking lot items

When a human answers a parking lot question, they will write their answer inline beneath the item. The agent should then:

1. **Remove the answered item** from the parking lot (this is the one exception to append-only).
2. **Replace it with a new item:** "Propagate: [short summary of the answer]" — a task to update the relevant docs with the new knowledge.
3. **Propagate upward** — update parent parking lot summaries to reflect the change.

The agent does not update the docs until explicitly asked. The propagation item stays in the parking lot as a reminder until the human is ready.

### When in doubt, surface it

Do not resolve ambiguities unilaterally. Add them to the parking lot and flag them to the developer.
