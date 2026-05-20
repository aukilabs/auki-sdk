# Changelog — auki-logs-py

Append-only changelog for this crate. See [CLAUDE.md](../../../CLAUDE.md) for the format and propagation rules.

Latest entry on top.

---

### Nils's codex · May 20, HKT, 2026

**Package relocated to `bindings/python/auki-logs-py`.** The Python package moved from `crates/auki-logs-py` to `bindings/python/auki-logs-py` with no package-name, module-name, or runtime behavior changes. Cargo workspace membership and local path dependencies now point at the new location.

### Nils's codex · May 19, HKT, 2026

**Retained stream sources for Python producers.** `auki_logs.Log.stream_source(...)` now returns a frozen `StreamSource` metadata object carrying the log root, sensor/clock/frame hashes, and a `payload_kind` discriminator (`camera`, `pointcloud`, `joint_encoders`, or `audio`). Apps pass this object to `auki_network.cluster.StreamDecision.accept_source(source)` instead of constructing stream manifests or choosing typed accept factories.

The cross-extension bridge is a named SDK-internal PyCapsule, `auki_logs_py::stream_source::v1`, whose payload is a Rust `RetainedStreamSource`. The logs binding still stores opaque bytes; the network binding owns retained-log tailing and payload decoding. Python tests cover metadata getters, optional frame metadata, payload-kind validation, and capsule naming.

### broodsugar's claude · May 9, 13:30 HKT, 2026

**Crate scaffolding + Python surface for [`auki-logs`](../auki-logs)'s `Log<T>` framing primitive.** Closes [`detectors`](https://github.com/aukilabs/detectors) phase-2 blocker #4 at the bytes level — the ESL detector (Python) can now tail an input sensor log and append to an output detection log without leaving Python.

**Surface:**

```python
import auki_logs

# Write side
log = auki_logs.Log.open("/path", manifest_dict)        # or `with Log.open(...) as log:`
log.append(timestamp_ns, payload_bytes)
log.flush(); log.set_retention(ns); log.manifest()
log.close()

# Read snapshot
reader = auki_logs.Log.read("/path")
reader.entries() / reader.segment_starts() / reader.manifest()

# Tail (read side of the keystone)
tail = auki_logs.Log.tail("/path").with_poll_interval(10)
for entry in tail:                                       # blocking
    process(entry.timestamp_ns, entry.payload)
# OR
entry = tail.try_next()                                  # non-blocking; None if no entry ready
```

**Encoding stance — opaque bytes only.** Mirrors the Rust crate's `LogPayload` philosophy: there's no equivalent Python trait. Callers pass `bytes` for `payload`; readers receive `bytes`. Decoding the prost bytes into typed `auki-datatypes` is the consumer's responsibility (hand-roll for now; `betterproto` codegen lands at [Step 9 of the migration sprint](../auki-datatypes/src/sprint.md)).

**Internal mechanism.** A tiny `RawBytes(Vec<u8>)` newtype with an identity-encoding `LogPayload` impl satisfies the `Log<T>` generic bound and presents the opaque-bytes surface in Python. Cross-language byte equality is preserved because `auki-logs`'s segment format is encoder-agnostic — what the producer hands to `append()` lands on disk byte-for-byte, regardless of language.

**Manifest seam.** Python `dict` ↔ Rust `serde_json::Value` round-trips via Python's stdlib `json` module — `json.dumps` on entry, `json.loads` on exit. Avoids hand-coding a pydict-to-serde walker; cost is negligible for ~1KB manifests.

**GIL.** `Log.open`, `append`, `flush`, `read`, `entries`, `set_retention`, `tail` open, and `__next__` (blocking) all release the GIL via `Python::allow_threads`. Non-blocking calls (`try_next`, getters) hold it.

**Errors.** `Error::Io` → `OSError`, `Error::Payload` / `Manifest` / `Format` → `ValueError` with prefix. Closed-log access → `RuntimeError`. Same shape as `auki-network-py`.

**Tests:**
- Rust-side (`cargo test -p auki-logs-py`): 3 — `RawBytes` `LogPayload` round-trip, empty-payload round-trip, cross-encoder seam stub.
- Python-side (`pytest python_tests/`): 13 — surface (4), round-trip (4), tail (4), end-to-end fake-detector smoke (1).

**Build pipeline.** `abi3-py38` via PyO3 0.22 + maturin (matches [`auki-network-py`](../auki-network-py) and [`auki-identity-py`](../auki-identity-py)). Default-empty Cargo features so `cargo test` against the rlib works; `extension-module` enabled by maturin via `[tool.maturin]` for native-extension builds.

**Out of scope** (filed in [`parking_lot.md`](parking_lot.md) and [`src/sprint.md`](src/sprint.md)):
- `betterproto`-generated `auki-datatypes` Python types — Step 9 of the migration sprint.
- Companion `auki-layout-py` + `auki-manifests-py` (path / manifest helpers) — sibling follow-up PR.
- Cross-language locked vectors at the segment-file level — filed at the Rust side, lands when concrete drift becomes painful.
- Type stubs (`auki_logs.pyi`) — track `auki-network-py`'s parallel discussion.
- PyPI distribution — track `auki-identity-py` and `auki-network-py`.

Will land in v0.0.26.
