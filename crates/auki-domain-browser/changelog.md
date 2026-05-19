# Changelog — auki-domain-browser

Append-only timeline of changes for the browser Domain peer adapter. Latest entry on top.

---

### Nils's codex · May 19, HKT, 2026

Added Discovery HTTP domain listing for browser peers. `listDomains(...)` maps `/clusters` responses into the Park-compatible `DomainSummary` contract and returns UI-friendly `discovery_unreachable` / `domain_list_failed` errors.

### Nils's codex · May 19, HKT, 2026

Added the browser identity seed storage seam and `shortPeerId(...)` helper. The seed helper persists exactly 32-byte seeds through an injectable store, rejects malformed stored/generated seeds, and formats visible peer ids from the final six characters.

### Nils's codex · May 19, HKT, 2026

Added the Park-compatible browser peer contract, structured result/error helpers, package exports, and the `window.aukiBrowserPeer.createPeer()` global installer. The focused installer test and TypeScript build pass for the first importable package surface.

### Nils's codex · May 19, HKT, 2026

Created the `auki-domain-browser` package scaffold for Park's browser-peer Milestone 0 handoff. The first tranche is explicitly an SDK boundary and identity/Discovery shell; real browser transport and audio remain follow-up work.
