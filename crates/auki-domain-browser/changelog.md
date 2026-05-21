# Changelog — auki-domain-browser

Append-only timeline of changes for the browser Domain peer adapter. Latest entry on top.

---

### Nils's codex · May 22, HKT, 2026

`createBrowserDomainPeer` now keeps fallback media presence snapshots coherent when an injected SDK session returns successful publish/listen operations but does not own participant observation. Successful audio publication marks mic availability/capture/publication locally; successful subscribe/unsubscribe updates the selected remote stream intent. This keeps Park's UI state compatible with the new wasm browser session and with focused tests.

Tests: `npm --prefix crates/auki-domain-browser test -- peer.test.ts`, `npm --prefix crates/auki-domain-browser run build`.


### Codex · May 21, HKT, 2026

`createBrowserDomainPeer(...)` now attempts to resolve a runtime browser SDK peer from `window.aukiBrowserPeer` when no explicit session is supplied. That enables Park/browser startup after the SDK adapter is installed, while still falling back to `transport_unavailable` behavior if the global factory is missing or fails. Added coverage for global-factory delegation and failure fallback.

### Nils's codex · May 21, HKT, 2026

Aligned the browser Domain contract with the current SDK stream vocabulary. `SensorKind` now uses `camera`, `point_cloud`, `joint_encoders`, `audio`, `detection`, and `unknown`; `StreamState` now uses `off`, `idle`, `connecting`, `connected`, `reconnecting`, `declined`, and `error`; exported constants and tests pin the vocabulary for Park.

### Nils's codex · May 19, HKT, 2026

Hardened the browser package after code review. The emitted ESM entry now imports correctly under NodeNext semantics, `npm run smoke:import` verifies the built package entry, Discovery malformed-payload errors are classified as `domain_list_failed`, and all transport-backed peer methods are pinned to fail closed until browser transport exists.

### Nils's codex · May 19, HKT, 2026

Added the first-tranche `createBrowserDomainPeer(...)` shell. It reports its peer id, emits an idle unjoined participant snapshot immediately, delegates `listDomains(...)` to Discovery mapping, accepts local metadata/sensor declarations as no-ops, and returns `transport_unavailable` for join/create/sensor stream operations until browser SDK transport exists.

### Nils's codex · May 19, HKT, 2026

Added Discovery HTTP domain listing for browser peers. `listDomains(...)` maps `/clusters` responses into the Park-compatible `DomainSummary` contract and returns UI-friendly `discovery_unreachable` / `domain_list_failed` errors.

### Nils's codex · May 19, HKT, 2026

Added the browser identity seed storage seam and `shortPeerId(...)` helper. The seed helper persists exactly 32-byte seeds through an injectable store, rejects malformed stored/generated seeds, and formats visible peer ids from the final six characters.

### Nils's codex · May 19, HKT, 2026

Added the Park-compatible browser peer contract, structured result/error helpers, package exports, and the `window.aukiBrowserPeer.createPeer()` global installer. The focused installer test and TypeScript build pass for the first importable package surface.

### Nils's codex · May 19, HKT, 2026

Created the `auki-domain-browser` package scaffold for Park's browser-peer Milestone 0 handoff. The first tranche is explicitly an SDK boundary and identity/Discovery shell; real browser transport and audio remain follow-up work.

### Nils's codex · May 21, HKT, 2026

`createBrowserDomainPeer` now makes successful wasm joins visible. When a wasm-shaped session returns join metadata, the wrapper parses the membership JSON, records the joined domain and Manager peer, preserves local participant metadata and declared sensors, and emits a stable `observeParticipants()` snapshot for Park.
