# Changelog — bindings

One-line summaries of changes in language binding packages. Detailed entries live in the most-specific binding package `changelog.md` files and propagate up to the root timeline.

Latest entry on top.

---

### Nils's claude · May 22, HKT, 2026

`bindings/swift/auki-network-swift` PR B landed: expanded to full v0 networking surface (runtime + 5-payload stream surface + callback interfaces). Stage 1's hand-wrapped Discovery surface replaced by upstream-annotated re-exports. See [`swift/auki-network-swift/changelog.md`](swift/auki-network-swift/changelog.md) and [`swift/changelog.md`](swift/changelog.md).

### Nils's claude · May 21, 15:41 HKT, 2026

**`auki-identity-swift` added to [`bindings/swift/`](swift/changelog.md).** PR A of Spec 1. Thin UniFFI scaffolding host for `Wallet` + `PeerIdentity` via the upstream `swift-bindings` cargo feature; surface at v0 is the minimum the iosapp Keychain helper needs.

### Nils's codex · May 21, HKT, 2026

**Python bindings updated for the SDK stream naming cleanup.** See [`bindings/python`](python/changelog.md) for package-level propagation of the `CameraFrame` / `DetectionFrame` / `Camera` vocabulary.

### Nils's claude · May 20, 13:31 HKT, 2026

**Swift bindings family added under [`bindings/swift`](swift/changelog.md).** `auki-network-swift` relocated from `crates/auki-network-swift` to `bindings/swift/auki-network-swift` to follow the same per-language convention introduced for Python by PR #156. Package name, lib name, surface, runtime behavior unchanged; only paths and relative doc links moved.

### Nils's codex · May 20, HKT, 2026

**Python packages moved under [`bindings/python`](python/changelog.md).** The `auki-*-py` package family left `crates/` for the language-binding hierarchy with package names, Python module names, and runtime behavior preserved.
