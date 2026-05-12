# Sprint — `auki-domain-py`

## Current

Phase 1 (this PR) — initial crate ship with `init_domain` + `DomainHandle` + typed exceptions. Lib-name-collision blocker on cross-py-crate type sharing dictates the surface scope.

## Next

In order:

1. **Resolve the lib-name collision** ([`parking_lot.md`](../parking_lot.md) #1, recommended path B). Verify that renaming `auki-network-py`'s `[lib] name` to `auki_network_py` keeps maturin's wheel build clean (cdylib → Python module rename works via `[tool.maturin] module-name = "auki_network"`). If it does, every cross-py-crate dep follow-up unblocks at once.
2. **Wire `stream_provider`** ([`parking_lot.md`](../parking_lot.md) #1). Once #1 above lands: add `stream_provider: Option<Py<PyAny>>` kwarg to `init_domain`, route through `auki-network-py`'s `build_stream_provider` (promote `pub(crate)` → `pub` in that crate), pass result instead of `decline_all_streams()`. Producer daemons (BoosterApp, Sentinel) become fully functional on v0.0.33.
3. **Wire `handle.update_cluster_doc(new_doc)`** ([`parking_lot.md`](../parking_lot.md) #3). OR wait for the SDK-side "ClusterRuntime owns its SSE subscription internally" tightening to collapse this entirely (filed in [`auki-network/parking_lot.md`](../../auki-network/parking_lot.md)). Whichever lands first wins.
4. **Surface `DomainAlreadyExists.existing` as a Python `ClusterDoc`** ([`parking_lot.md`](../parking_lot.md) #4). Same lib-name-collision blocker as #2; piggybacks on the resolution there.

## Long-term

When `auki-domain` grows the Manager-role / heartbeat / failover state (Greenland T2–T7, T10–T13 — see [`auki-domain/src/sprint.md`](../../auki-domain/src/sprint.md)), this crate gains the corresponding Python surface:
- `handle.is_manager()` / `handle.manager_peer_id()`.
- `handle.send_heartbeat()` (or that becomes runtime-internal).
- `handle.join_request_callback(...)` for Manager-side admission control.
- `handle.failover_trigger()` for the operator-driven failover path.

Wait until those land on the Rust side before designing the Python wrap.
