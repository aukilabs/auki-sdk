# Parking lot — root

Open architectural questions and cross-cutting design decisions for the Auki SDK as a whole. Crate-specific questions live in each crate's `parking_lot.md`; cross-crate questions live in [`crates/parking_lot.md`](crates/parking_lot.md).

When a question is answered inline, an agent will replace the item with a "Propagate: …" task — see [CLAUDE.md](CLAUDE.md) for the workflow.

---

## README casing convention

Outer crate READMEs are `README.md` (uppercase). Inner per-crate implementation status is in `src/readme.md` (lowercase). [CONTRIBUTING.md](CONTRIBUTING.md) shows uppercase for both. Should the inner files be renamed for consistency, or is the casing intentional (e.g. "outer is the public spec, inner is the working notes")?

## Outer README framing — "aspirational" vs "current state"

[CONTRIBUTING.md](CONTRIBUTING.md) describes the outer crate `README.md` as the **aspirational spec** ("what this component should be") while `src/README.md` is **what is actually implemented today**. In practice the outer crate READMEs we just wrote describe what's implemented today, since spec and reality match. As the SDK grows and ambitions outpace implementation, the two framings diverge — which one is the outer README?

## Cross-language conformance vectors

`auki-hash` publishes locked conformance vectors so any reimplementation can be validated. Should `auki-jcs`, `auki-logs`, `auki-registry`, and `auki-time-transforms` also publish locked vectors? Concretely: a `tests/cross_language/` directory with golden bytes any port must reproduce. Boosterapp's Python sidecar is already a de facto second implementation — vectors would catch drift automatically.

## Glossary.md

[CLAUDE.md](CLAUDE.md) references a root-level `Glossary.md` for "definitions of all key terms." It doesn't exist yet. Who drafts it, and what's the seed list of terms (Domain, Cluster, Map, Frame, Pose Log, etc.)?

## Python bindings strategy

Listed under "Not yet implemented" in [README.md](README.md). Path forward options: (a) PyO3 wrapper exposing `auki-logs`, `auki-registry`, `auki-time-transforms`; (b) pure-Python re-implementation per the on-disk specs; (c) bless boosterapp's existing Python sidecar as the official binding. Each has different drift-risk, packaging, and effort tradeoffs.

---

## Subfolder summary

- [`crates/`](crates/parking_lot.md) — schema versioning coordination; convention scaffolding gaps (sprint.md, changelog.md)
