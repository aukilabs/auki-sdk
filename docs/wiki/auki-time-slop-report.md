# auki-time Slop Report

**Task:** t_17f5bd50
**Date:** 2026-06-05
**Scope:** auki-time crate + cross-crate time sync surface (auki-session, auki-manifests, auki-datatypes, auki-registry)

## Items

### 1. Unnecessary public re-export of auki-logs
- **File path + line range:** crates/auki-time/src/lib.rs:27
- **Category:** dead_code
- **Severity:** low
- **Concrete description:** `pub use auki_logs;` makes the entire auki-logs crate available as `auki_time::auki_logs`. Internal code uses the direct `auki_logs::` dependency path. No external call sites reference `auki_time::auki_logs`. The re-export increases the public API surface and hides crate boundaries (also noted as "Potential slop" in the api-map handoff). Consumers should depend on auki-logs directly.

No other matches for dead_code, misleading_comment, todo_placeholder, inconsistent_naming, copy_paste, or stale_reference across the scanned files.

## Proposed SDK Kanban ticket (pending explicit user approval before filing)
- **Title:** auki-time: remove unnecessary `pub use auki_logs;` re-export
- **File path:** crates/auki-time/src/lib.rs
- **Line range:** 27
- **Category:** dead_code
- **Severity:** low
- **Description:** The `pub use auki_logs;` on line 27 exposes the full auki-logs crate under the auki_time namespace. It is unused by any consumer code (search for auki_time::auki_logs returns only the api-map reference). Remove the line; internal uses already resolve via the Cargo dependency. This reduces public surface and clarifies crate ownership.

(Note: Per documenter rules, this list was compiled before any GitHub ticket creation. No tickets filed yet. Will track after approval.)

## Documentation actions taken
- Created docs/wiki/auki-time.md (9733 bytes) covering all public items from the api-map, time sync mechanism, usage notes, edge cases, and migration notes.
- Patched docs/wiki/_Sidebar.md to link the new page under For SDK Consumers.
- The new doc surfaces the Step-6 migration shape and the re-export note so users are not surprised by the current surface.

No slop required code changes by documenter (per SDK rule). Code is otherwise clean: excellent test coverage, no TODOs/FIXMEs in production paths, consistent naming, no copy-paste or stale references.