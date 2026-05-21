# Changelog — docs/superpowers

Append-only timeline of Superpowers design artifacts. Latest entry on top.

---

### Nils's codex · May 21, HKT, 2026

Updated the browser Domain adapter plan/spec under [`plans/`](plans/changelog.md) and [`specs/`](specs/changelog.md) so the browser contract cannot drift from the current SDK sensor-kind and stream-state vocabulary.

### Nils's codex · May 20, HKT, 2026

Updated the browser Domain peer symmetry plan under [`plans/`](plans/changelog.md) so `auki-network-browser-wasm` and `auki-domain-browser` are bindings/facades over shared Rust SDK logic rather than parallel browser implementations.

### Nils's codex · May 20, HKT, 2026

Reframed the browser Domain WebRTC implementation plan under [`plans/`](plans/changelog.md) so browser peers are full role-symmetric Domain peers, including Manager eligibility, with reachability handled as SDK transport state.

### Nils's codex · May 19, HKT, 2026

Added the browser Domain WebRTC join implementation plan under [`plans/`](plans/changelog.md), following the passing browser-to-native probe with production Manager WebRTC advertisement and browser join/info wiring.

### Nils's codex · May 19, HKT, 2026

Added the browser WebRTC probe stream implementation plan under [`plans/`](plans/changelog.md), sequencing the first browser-to-native SDK-owned protocol stream after the wasm libp2p feature compile probe passed.

### Nils's codex · May 19, HKT, 2026

Added the wasm libp2p browser transport compile-probe implementation plan under [`plans/`](plans/changelog.md), sequencing the first measurable SDK browser networking spike before native dial work.

### Nils's codex · May 19, HKT, 2026

Added the wasm libp2p browser transport spike spec under [`specs/`](specs/changelog.md), turning the browser transport question into a rust-libp2p Wasm proof plan before Domain join/audio.

### Nils's codex · May 19, HKT, 2026

Added the browser Domain peer adapter implementation plan under [`plans/`](plans/changelog.md), keeping first-tranche SDK package work separate from the later browser transport/audio plan.

### Nils's codex · May 19, HKT, 2026

Added the browser Domain peer adapter spec under [`specs/`](specs/changelog.md), documenting the SDK-side package and transport work needed for Park's browser-peer Milestone 0.

### Nils's codex · May 19, HKT, 2026

Added the native pointcloud SDK refactor design under [`specs/`](specs/changelog.md), documenting the approved breaking pointcloud contract for implementation planning.
