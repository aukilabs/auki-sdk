# Agents Guide — Auki SDK

This file is for AI agents reading this repo.

## What this repo is

This is the design foundation for the Auki SDK — a human and AI readable wiki and task repository that defines how the network works, what it is made of, and what needs to be built.

## Core concept

The SDK has two core operations: `convert_time` (agree on time) and `convert_pose` (communicate across coordinate systems). Everything else — clusters, maps, detectors, credits — is infrastructure that produces the transforms these two operations consume. When reading or writing any document in this repo, keep this framing in mind.

## Folder convention

The repo uses the current-state / aspirational split described in
[`CONTRIBUTING.md`](CONTRIBUTING.md):

```
VISION.md            ← The aspirational repo-level spec
README.md            ← What is actually implemented today
crates/<component>/
  VISION.md          ← Aspirational component spec, when present
  README.md          ← Current component state and public surface
  src/
    *.rs             ← Source code
```

When reading a component, check the repo overview first, then the
component `README.md`, and then the source. Use `VISION.md` files for
intended direction, not as proof that behavior is already implemented.

## Root files

- `README.md` — start here. Current repo state, crate map, and component index.
- `VISION.md` — aspirational spec and longer-term direction.
- `CONTRIBUTING.md` — folder convention, project board flow, and git hygiene.
- `AGENTS.md` — this file. Rules for AI agents.
- `Glossary.md` — definitions of all key terms.
- `skills/auki-sdk-app-builder/SKILL.md` — public app-building skill for agents using the Auki SDK.

## SDK app-building skill

When building public applications, demos, integrations, robot producers, or tools with the Auki SDK, read [`skills/auki-sdk-app-builder/SKILL.md`](skills/auki-sdk-app-builder/SKILL.md) before implementing. The skill explains what the SDK is supposed to handle, how to inspect SDK surfaces before coding, and how to avoid hand-rolling SDK concepts such as resources, registries, streams, clocks, payloads, and `auki-geometry` transforms in app code.

## Rules

### Project board, questions, and follow-ups

All project management is done through the SDK GitHub Project and issues.
Do not create local `parking_lot.md`, `changelog.md`, or `src/sprint.md`
files unless the developer explicitly asks for them.

When you find an ambiguity, unresolved design decision, bug, or follow-up
that should be tracked, surface it to the developer and propose a GitHub
Project item instead of opening one autonomously. Include:

- type: Question / Task / Exploring
- title
- one-paragraph body
- target column

Follow the more detailed board rules in [`CLAUDE.md`](CLAUDE.md#project-board).

### When in doubt, surface it

Do not resolve ambiguities unilaterally. Flag them to the developer and
propose filing a GitHub Project Question when the decision should be tracked.
