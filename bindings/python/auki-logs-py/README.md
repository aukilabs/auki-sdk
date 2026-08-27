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

# Producer sources preserve replay + live log semantics in authenticated
# Domain streams (`auki-domain-py`). Their identity fields are frozen and
# readable by application adapters as well as the Domain binding.
sensor_source = log.stream_source(...)  # payload_kind also accepts "scalar"
map_source = log.map_stream_source(
    resource_id="voxel/world",
    map_peer_id=peer_id,
    map_id="voxel/world",
    map_hash=map_hash,
    clock_peer_id=peer_id,
    clock_id="sdk_clock",
    clock_hash=clock_hash,
)
map_source.resource_id, map_source.map_peer_id, map_source.map_id
map_source.map_hash, map_source.clock_peer_id

reader = log.read()
for entry in reader.entries():
    entry.timestamp_ns, entry.payload  # both read-only

for entry in log.tail(poll_interval_ms):  # blocking iterator; drop to stop
    ...
```

Types: `Log`, `LogReader`, `TailIter`, `Entry`.

## Depends on

- [`auki-logs`](../../../crates/auki-logs) — Rust crate it wraps.
