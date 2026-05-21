# Changelog — docs

Append-only timeline of documentation changes under `docs/`. Latest entry on top.

---

### Nils's codex · May 21, HKT, 2026

Updated the browser Domain adapter planning docs so Park and the browser SDK package use the same current SDK vocabulary: `audio`, `camera`, `point_cloud`, `joint_encoders`, `detection`, plus UI stream states `declined` and `error`.

### Nils's codex · May 20, HKT, 2026

Updated the browser Domain plan to prevent browser/native SDK drift: browser crates are now bindings/facades over shared Rust `auki-network` and `auki-domain` logic, with runtime-specific code limited to concrete wasm/browser constraints.

### Nils's codex · May 20, HKT, 2026

Rewrote the browser Domain WebRTC plan around true peer symmetry: browser peers can be Managers, Discovery records PeerIds rather than platform classes, and reachability is an SDK transport concern.

### Nils's codex · May 19, HKT, 2026

Added the Superpowers browser Domain WebRTC join implementation plan after the browser probe smoke passed, covering production Manager WebRTC advertisement, browser wasm raw SDK substreams, and `auki-domain-browser` join wiring.

### Nils's codex · May 19, HKT, 2026

Added the Superpowers browser WebRTC probe stream implementation plan and propagated the docs-level changelog chain for the native listener plus browser wasm dial proof.

### Nils's codex · May 19, HKT, 2026

Added the Superpowers wasm libp2p browser transport compile-probe implementation plan and propagated the docs-level changelog chain for the first SDK browser networking spike slice.

### Nils's codex · May 19, HKT, 2026

Added the Superpowers wasm libp2p browser transport spike spec and propagated the docs-level changelog chain for the SDK-owned browser networking proof.

### Nils's codex · May 19, HKT, 2026

Added the Superpowers browser Domain peer adapter implementation plan and propagated the docs-level changelog chain for the first SDK package tranche.

### Nils's codex · May 19, HKT, 2026

Added the Superpowers browser Domain peer adapter design spec and propagated the docs-level changelog chain for the SDK package Park needs to load real browser peers.

### Nils's codex · May 19, HKT, 2026

Added the Superpowers native pointcloud design spec and propagated the docs-level changelog chain for the SDK pointcloud refactor planning artifact.
