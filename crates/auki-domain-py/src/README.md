# `auki-domain-py/src` — what's implemented

## Files

- [`lib.rs`](lib.rs) — single-file crate. PyO3 module entry point + `init_domain` Python function + `DomainHandle` pyclass + five typed Python exception classes + the duck-typed `build_participant_provider` adapter + Rust unit tests. ~600 lines including tests.

## Status

| Feature | Status |
|---------|--------|
| `init_domain` Python function | ✅ shipped |
| `DomainHandle` pyclass (`.identity`, `.peers()`, `.shutdown()`) | ✅ shipped |
| Typed Python exceptions (5 classes) | ✅ shipped |
| `participant_provider` duck-typed callable | ✅ shipped |
| `stream_provider` Python callable | ❌ parking_lot #1 |
| `handle.open_*_stream()` consumer methods | ❌ parking_lot #2 |
| `handle.update_cluster_doc()` SSE feed | ❌ parking_lot #3 |
| `DomainAlreadyExists.existing` ClusterDoc | ❌ parking_lot #4 |

See [`sprint.md`](sprint.md) for what's next.
