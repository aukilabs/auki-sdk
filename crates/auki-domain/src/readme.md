# auki-domain - implementation status

What is implemented today. See [`../README.md`](../README.md) for the crate-level spec.

## Files

- [`lib.rs`](lib.rs) - feature-gated module wiring and public re-exports.
- [`core.rs`](core.rs) - binding-free Rust API, module declarations, re-exports, and shared JSON adapters.
- [`ffi.rs`](ffi.rs) - native UniFFI records/errors/functions and bounded `DomainClusterManager` facade.
- [`wasm.rs`](wasm.rs) - wasm-bindgen wrappers for browser-safe membership JSON and election helpers.
- [`cluster_membership.rs`](cluster_membership.rs) - `ClusterMembership`, `ClusterMember`, membership JSON shape, filename helper, admission ordering.
- [`cluster_manager.rs`](cluster_manager.rs) - `ClusterManager`, `ClusterTarget`, daemon info, sensor/resource catalog provider traits, Discovery bootstrap logic, Manager/member state, join/liveness/membership/info/resources/sensors tasks, heartbeat-derived clock-sync forwarding, stream opener, shutdown, and election helper.
- [`stream_manifest.rs`](stream_manifest.rs) - producer-side `StreamManifestBuilder` that derives accept metadata from local Sensor / Frame registries.
- [`bin/uniffi-bindgen.rs`](bin/uniffi-bindgen.rs) - crate-local UniFFI CLI entry point used by binding generation.
- [`../bindings.toml`](../bindings.toml) - binding generation policy for Python, Swift, and JavaScript.

## Implemented

- `ClusterMembership::new`, `filename`, and `admit`.
- `ClusterManager::list_clusters`, `bootstrap`, `create_cluster`, and `join_cluster`.
- `ClusterTarget::{create, join, join_or_create, most_recent_or_create}`.
- Manager admission through `/auki/join/0.0.1`.
- Membership gossip through `/auki/membership/0.0.1`, including the Manager peer id to converge handoff broadcasts.
- Manager-star heartbeat/liveness detection through `/auki/heartbeat/0.0.1`, with topology and timeout semantics owned here rather than in `auki-network`; raw carrier close is not treated as semantic death until the heartbeat timeout expires.
- Heartbeat frames timestamped from the daemon's explicit session clock id/hash and local session monotonic elapsed nanoseconds.
- Initial Managers include optional heartbeat domain-clock metadata declaring `<cluster_name>/domain-clock` backed by their own session clock with offset `0`; joiners omit this metadata until they are promoted and can prove their inherited domain offset.
- Heartbeat domain-clock metadata from the backing peer is stored locally. `domain_clock_estimate()` composes it with `auki-time` peer-clock estimates and returns explicit unavailable errors until both pieces exist; `domain_time_now()` converts the current session-clock reading through that estimate and reports overflow explicitly.
- Heartbeat timing observations accepted from `auki-network` without doing NTP math in `ClusterManager`; raw NTP sample events are forwarded into an `auki-time::ClockSyncHandle` and exposed through read-only `clock_sync_estimate` / `clock_sync_estimates` accessors.
- Manager election and Discovery `rotate_manager` handoff.
- Manager -> Discovery `liveness_check` loop every `LIVENESS_CHECK_INTERVAL` (1 second).
- SDK-owned `ParticipantInfo` generation plus `/auki/info/0.0.1` fetches, with `session_now_ns` and session clock id/hash sourced from `auki_time::SessionClock`.
- Resource catalog provider registration plus `/auki/resources/0.0.1` fetches, auto-lifting sensor catalog rows into `sensor_stream` resources and accepting producer-supplied `transform_edge` rows.
- Sensor catalog provider registration plus `/auki/sensors/0.0.1` fetches, including the detail request that can embed local Sensor / Frame Registry JSON by value.
- Registry app-root registration plus `/auki/registries/0.0.1` typed fetches for Sensor / Clock / Frame Registry entries.
- `StreamManifestBuilder::from_registry`, which constructs stream accept manifests from a producer's local registry and verifies exact frame references for spatial sensors.
- Cluster-handle `open_stream::<T>` delegating to `NetworkRuntime`.
- Shared-reference, idempotent `shutdown`.
- Shared JSON adapters for membership construction, filename/count reads, member admission, and deterministic successor election.
- Native generated bindings for Python and Swift through UniFFI. The exported `DomainClusterManager` bootstraps with an internally-built network runtime and exposes cluster state, manager admission, diagnostics, membership events, participant info JSON, domain time and clock estimates, catalog/resource/registry providers, catalog/resource/registry fetches, camera/detection byte streams, and shutdown through binding-safe records. Generated Swift hosts can either pass explicit advertised multiaddrs or call the auto-advertise bootstrap so the SDK resolves advertised listen addresses from a binding-safe listen-address list.
- Browser generated bindings through wasm-bindgen for membership/election helpers and domain DTO validation helpers. The generated JavaScript package also exports `AukiDomainClient`, which composes a `requestFramed`-compatible `auki-network` browser peer for domain request/response flows.

## Public Re-exports

`lib.rs` re-exports:

- `ClusterManager`
- `ClusterTarget`
- `ClusterMembership`
- `ClusterMember`
- `DaemonInfo`
- `ResourceCatalogProvider`
- `ResourceEntry`
- `ResourceKind`
- `ResourcePinholeIntrinsics`
- `ResourceQuat`
- `ResourceSpatialTransform`
- `ResourceVec3`
- `ResourcesRequest`
- `ResourcesResponse`
- `SensorStreamResource`
- `TransformEdgeResource`
- `SensorCatalogProvider`
- `SensorEntry`
- `SensorsRequest`
- `SensorsResponse`
- `SensorRegistryEntry`
- `ClockRegistryEntry`
- `ClockTransformEstimate`
- `DomainClockEstimate`
- `DomainClockEstimateUnavailable`
- `DomainTimeNowError`
- `FrameRegistryEntry`
- `RegistryKind`
- `StreamManifestBuilder`
- `LIVENESS_CHECK_INTERVAL`
- Error types for bootstrap/create/join/admit/fetch paths
- `elect_successor`
- `cluster_membership_new_json`
- `cluster_membership_filename_json`
- `cluster_membership_peer_count_json`
- `cluster_membership_admit_member_json`
- `elect_successor_json`
- `bootstrap_domain_cluster_manager`
- `bootstrap_domain_cluster_manager_auto_advertise`

## Timekeeping

`ClusterManager` constructs one SDK-owned `SessionClock` at create/join time using the local peer id and `DaemonInfo.session_id`. `ParticipantInfo.session_clock_id`, `session_clock_hash`, `session_now_ns`, heartbeat timestamps, and heartbeat domain-clock backing metadata come from that clock, not from caller-supplied `DaemonInfo.session_clock_id/hash`. Those input fields remain accepted for compatibility until the constructor surface can stop asking callers for session clock identity.

## Deferred

- Typed successor-token format and Discovery-side verification.
- SDK-managed relay reservation helper for non-LAN clusters.
- Possible demotion of direct `DiscoveryClient` usage after app migrations prove the SDK-fronted `ClusterManager` path.

## Verification

For implementation changes:

```bash
cargo test -p auki-domain --no-default-features
cargo test -p auki-domain
cargo check -p auki-domain --target wasm32-unknown-unknown --no-default-features --features wasm
python3 scripts/bindings/generate_bindings.py plan python auki-domain
python3 scripts/bindings/generate_bindings.py plan swift auki-domain
python3 scripts/bindings/generate_bindings.py plan javascript auki-domain
AUKI_PYTHON_NATIVE_TARGETS="$(rustc -vV | awk '/host:/ {print $2}')" just generate-python-bindings auki-domain
just generate-javascript-bindings auki-domain
just generate-swift-bindings auki-domain
swift build --package-path bindings/swift/auki-domain
python3 crates/auki-domain/bindings/python/smoke_full_domain.py
swift run --package-path crates/auki-domain/bindings/swift/SmokeFullDomain SmokeFullDomain
npm --prefix bindings/javascript/auki-domain test
DISCOVERY_URL=http://127.0.0.1:8080 cargo test -p auki-domain --test cluster_manager_integration -- --ignored
```
