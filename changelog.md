# Changelog — root

Append-only timeline of changes across the repo. Detailed entries land in the most-specific (leaf) `changelog.md`; one-liners propagate up through every parent to here. See [CLAUDE.md](CLAUDE.md) for the propagation rules and entry format.

Latest entry on top.

---

### broodsugar's claude · May 1, 15:22 HKT, 2026

Bootstrapped `changelog.md` at every folder level — root, `crates/`, and all seven crates. Prior history lives in git log; this changelog tracks changes from this point forward. Same PR also fixed an existing convention violation: open questions buried inside `tags.md` and `dataproducts.md` moved to root [`parking_lot.md`](parking_lot.md), where they belong per the project's parking-lot convention. Removed the now-resolved "changelog.md per-crate scaffolding missing" item from `crates/parking_lot.md`.
