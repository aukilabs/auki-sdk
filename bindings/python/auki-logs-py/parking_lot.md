# Parking lot — auki-logs-py

Open questions for the `auki-logs-py` crate. Cross-cutting questions live in the [root `parking_lot.md`](../../../parking_lot.md).

When a question is answered inline, an agent will replace the item with a "Propagate: …" task — see [CLAUDE.md](../../../CLAUDE.md) for the workflow.

---

## Cross-language locked vectors at the segment-file level

The Rust crate has locked wire-bytes vectors per prost type (in [`auki-datatypes`](../../../crates/auki-datatypes)) but no locked vector for the `auki-logs` segment file binary format itself — header (`AKLG` magic + version + reserved + `start_ns`) + entry framing (`timestamp_ns` + `payload_len` + payload bytes). The root [Cross-language conformance vectors gap](../../../parking_lot.md) flags `auki-logs` as a hole.

This crate makes the gap concrete: a Python writer's segment files must be byte-identical to a Rust writer's for the same `(timestamp_ns, payload)` sequence, OR consumers in either language reading each other's logs will trip silent bugs. Worth adding a locked vector that pins:

- The 16-byte header for `start_ns = 0` (or some fixed value).
- A two-entry sequence with deterministic `(timestamp_ns, payload)` pairs encodes to specific bytes.
- The XXH3-128 hash of those bytes.

Filed at the Rust side ([`auki-logs/parking_lot.md`](../../../crates/auki-logs/parking_lot.md)) when this crate started consuming the format. Add when a real cross-language drift hunt becomes painful.

## Type stubs (`auki_logs.pyi`)

The surface is small enough that IDE support isn't critical, but `auki-network-py`'s parking-lot also flags type stubs as a deferred item. When `auki-network-py`'s stubs land, follow the same pattern here.

## PyPI distribution policy

Same question as `auki-identity-py` and `auki-network-py`: how does this wheel ship to consumers (Park, future Sentinel, the ESL detector author)? Today every consumer builds from source via `maturin develop`. PyPI publishing requires a wheel-building CI matrix (manylinux, macOS, possibly Windows). Defer until a non-source-build consumer needs the wheel.

## Behavioral parity with the Rust tail iterator

The Python `Log.tail` mirrors the Rust `Log::<T>::tail` semantics: starts at current EOF, polls at 10ms, blocks `__next__`, non-blocking `try_next`. Edge cases tested in `python_tests/`: tail-from-EOF skips existing entries, blocking iterator yields entries in order from a concurrent writer thread, `try_next` returns `None` when no entry is ready. Rust-side edge cases (segment rollover, mid-write torn reads, segment eviction under the tailer) are inherited from `auki-logs`'s own test suite — they're handled in the underlying iterator, not in this wrapper. If a parity test reveals a difference (e.g. PyO3's GIL semantics changing the polling cadence), file a specific item then.

## `betterproto`-generated `auki-datatypes` Python types

Currently the consumer's responsibility — hand-roll prost or use a custom encoder. Step 9 of the [`auki-datatypes` migration sprint](../../../crates/auki-datatypes/src/sprint.md) lands the `betterproto` codegen. Until then, the natural seam between `auki-logs-py` and the rest of the SDK's Python surface stays unbridged. File-and-revisit when Step 9 lands.
