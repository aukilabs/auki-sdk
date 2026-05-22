# auki-logs-py

PyO3 bindings for [`auki-logs`](../../../crates/auki-logs)'s `Log<T>` framing primitive. Opaque-bytes payload — Python handles prost encode / decode itself (via [`auki-datatypes-py`](../auki-datatypes-py) or hand-rolled), matching the Rust crate's encoder-agnostic stance.

Lets Python producers and consumers (e.g. the ESL detector in [`detectors`](https://github.com/aukilabs/detectors)) participate in the segmented ring-buffer log on equal footing with Rust callers.

**Status:** Shipped.

## Public surface

```python
import auki_logs

log = auki_logs.Log.open(path, manifest_dict)
log.append(timestamp_ns, payload_bytes)
log.flush()
log.set_retention(retention_ns)
log.manifest()                       # dict

reader = log.read()
for entry in reader.entries():
    entry.timestamp_ns, entry.payload  # both read-only

for entry in log.tail(poll_interval_ms):  # blocking iterator; drop to stop
    ...
```

Types: `Log`, `LogReader`, `TailIter`, `Entry`.

## Depends on

- [`auki-logs`](../../../crates/auki-logs) — Rust crate it wraps.
