# auki-logs

Generic segmented append-only log primitive — a manifest header plus numbered segment files plus retention-based eviction. Every on-disk log type in the SDK (Sensor, Pose, TimeTransform, Detection) is a `Log<T>` over a different segment payload.

Encoder-agnostic via the `LogPayload` trait: consumers pick prost (the default for types from [`auki-datatypes`](../auki-datatypes), via a blanket impl), ciborium, or hand-rolled bytes. Segment rollover is time-based (`segment_duration_ns`); eviction is retention-based (`retention_ns`); the two are independent.

**Status:** Shipped.

## Public surface

- `Log<T>` — open / append / flush / iterate; configurable retention.
- `LogReader<T>` — read-only snapshot over the segment set.
- `Entry<T>` — `(timestamp_ns, payload)` pair.
- `LogPayload` (trait) — `encode` / `decode` hook the consumer fills in.
- `Error` — IO + decode failure cases.

## Depends on

- [`auki-jcs`](../auki-jcs) — for canonical manifest JSON.
