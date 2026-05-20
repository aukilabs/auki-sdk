# Sprint — auki-manifests-py

Current work and next steps. Spec: [outer `README.md`](../README.md).

## Now

Crate landed 2026-05-09 alongside [`auki-layout-py`](../../auki-layout-py) — pure-function wrappers around the four `build_*_log_manifest` builders in [`auki-manifests`](../../../../crates/auki-manifests). Closes the manifest-construction half of the [`detectors`](https://github.com/aukilabs/detectors) phase-2 Python ergonomics gap.

## Next

1. **`PoseSource::canonical_bytes` / `hash` Python wrappers** — the graduation primitives for moving `PoseSource` to a sibling registry. Re-expose when a Python consumer needs them; defer until a real SLAM/odometry producer brings substantial identity. Same trigger as the Rust-side parking-lot question.
2. **PyClass equivalents of the enums?** — currently the Python surface takes `dict` / `str` for `PoseSource` / `PoseWriterMode` / `TimeTransformSource`. PyClass would give type-safety at the cost of more types Python users have to learn. Defer until a consumer asks.
3. **Type stubs (`auki_manifests.pyi`)** — track [`auki-network-py`](../../auki-network-py)'s parallel discussion.
4. **Read-side parsers + validators** — typed `SensorLogManifest` / `PoseLogManifest` / etc. structs with `validate()`. Adds when [`auki-manifests`](../../../../crates/auki-manifests) itself grows the read-side (also filed there).

## Out-of-band

- Manifest field shapes themselves live in [`auki-manifests`](../../../../crates/auki-manifests). Changes there propagate here automatically — these wrappers don't introduce a second layer of conventions.
- The `intent` field that the [keystone](../../../../parking_lot.md) wants on every manifest builder is currently absent across the board. When the uniform rollout PR lands ([filed in the Rust crate's parking-lot](../../../../crates/auki-manifests/parking_lot.md)), the wrapper signatures here will need the matching arg.
