# Parking lot — auki-manifests-py

Open questions for the `auki-manifests-py` crate. Cross-cutting questions live in the [root `parking_lot.md`](../../../parking_lot.md).

When a question is answered inline, an agent will replace the item with a "Propagate: …" task — see [CLAUDE.md](../../../CLAUDE.md) for the workflow.

---

## `PoseSource::canonical_bytes` / `hash` graduation primitives

The Rust crate's [`PoseSource`](../../../crates/auki-manifests/src/lib.rs) carries `canonical_bytes()` and `hash()` for the future "graduate to a sibling registry" path. The Python wrapper doesn't expose them yet — Python consumers don't need them today, and the canonicalize-via-JCS + XXH3 dance is reproducible in pure Python if a consumer needs to.

Re-expose if a real Python consumer needs the primitives (typically when graduating `PoseSource::Slam` or similar to a registry). Track [`auki-manifests`](../../../crates/auki-manifests/parking_lot.md)'s graduation discussion.

## PyClass equivalents of the enums?

Currently the Python surface takes:

- `PoseSource` as `dict` (`{"kind": "ros2_tf", "publishers": [...]}`)
- `PoseWriterMode` as `str` (`"rigid"` / `"movable"`)
- `TimeTransformSource` as `dict` (`{"kind": "local_clock_read"}`)

This sidesteps PyClass complexity but loses type-safety at the Python boundary. A typo in `"rigid"` → `"riged"` only surfaces as a `ValueError` at runtime; if Python had a `PoseWriterMode.Rigid` enum, the typo would surface as an `AttributeError` at definition time.

Lean: stay with dicts/strings — the dict shape is exactly what consumers serialize to anyway, and the validation cost is one parse per call (negligible at manifest-build frequency). Revisit if a consumer asks for typed Python enums.

## Type stubs (`auki_manifests.pyi`)

Same question as `auki-layout-py`. Track [`auki-network-py`](../auki-network-py/parking_lot.md)'s parallel discussion.

## PyPI distribution policy

Same as every other `*-py` crate. Defer until a non-source-build consumer needs the wheel.
