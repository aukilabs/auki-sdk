# Changelog — bindings

One-line summaries of changes in language binding packages. Detailed entries live in the most-specific binding package `changelog.md` files and propagate up to the root timeline.

Latest entry on top.

---

### Nils's claude · May 20, 13:31 HKT, 2026

**Swift bindings family added under [`bindings/swift`](swift/changelog.md).** `auki-network-swift` relocated from `crates/auki-network-swift` to `bindings/swift/auki-network-swift` to follow the same per-language convention introduced for Python by PR #156. Package name, lib name, surface, runtime behavior unchanged; only paths and relative doc links moved.

### Nils's codex · May 20, HKT, 2026

**Python packages moved under [`bindings/python`](python/changelog.md).** The `auki-*-py` package family left `crates/` for the language-binding hierarchy with package names, Python module names, and runtime behavior preserved.

