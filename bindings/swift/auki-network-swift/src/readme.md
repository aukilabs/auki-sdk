# `auki-network-swift/src/`

Implementation status for [`auki-network-swift`](../README.md). Honest about what is real today.

## Files

- [`lib.rs`](lib.rs) — the full v0 binding: custom-type registrations for `PeerId`/`Multiaddr`, the 7 Swift-side traits/enums/records, `spawn_for_swift` orchestrator, the 5 source-stream adapter functions, and `swift_provider_to_upstream` adapter.
- [`bin/uniffi-bindgen.rs`](bin/uniffi-bindgen.rs) — host Swift-codegen entry point, gated behind the `cli` feature.

## What works today

### Custom-type registrations

`PeerId` and `Multiaddr` are registered via `uniffi::custom_type!` with the `remote` keyword (UniFFI 0.31's mechanism for foreign-crate types that can't be annotated directly). Both cross the FFI as canonical strings.

### Swift-side types

- **`SwiftPeerLivenessEvent`** — 3-variant v0 UniFFI Enum (Connected, Disconnected, HeartbeatAlive). The two heartbeat-detail upstream variants (HeartbeatReceived, HeartbeatNtpSampleObserved) are dropped by the drain task inside `spawn_for_swift`.
- **`PeerLivenessListener`** — UniFFI callback-interface trait; one method `on_event(event: SwiftPeerLivenessEvent)`.
- **`HeartbeatTimestampProvider`** — UniFFI callback-interface trait; 4 methods: `clock_id()`, `clock_hash()`, `now_ns()`, `domain_clock_bytes()`. Note: `HeartbeatDomainClock` is JSON-encoded (lives in `auki_network::heartbeat_protocol`); `domain_clock_bytes` carries `serde_json`-encoded bytes.
- **`SwiftStreamDecision`** — UniFFI Enum; 6 variants: `AcceptAudio(manifestBytes)`, `AcceptCamera(manifestBytes)`, `AcceptPointCloud(manifestBytes)`, `AcceptJointEncoders(manifestBytes)`, `AcceptDetection(manifestBytes)`, `Decline(reasonBytes)`.
- **`SwiftStreamProvider`** — UniFFI callback-interface trait; two-call protocol: `dispatch_decision(peerId, requestBytes) -> SwiftStreamDecision`, then on each Accept variant the matching `*Source(peerId, requestBytes) -> Box<dyn Swift*Source>`.
- **`SwiftAudioSource`**, **`SwiftCameraSource`**, **`SwiftPointCloudSource`**, **`SwiftJointEncodersSource`**, **`SwiftDetectionSource`** — UniFFI callback-interface traits; each exposes `next_item() -> Result<Option<StreamItem>, SwiftSourceError>`.
- **`StreamItem`** — UniFFI Record `{ timestamp_ns: i64, payload_bytes: Vec<u8> }`.
- **`SwiftSourceError::Producer { message: String }`** — UniFFI Error; `String` as a direct throw type is rejected by UniFFI 0.31, hence the wrapper.
- **`SpawnSwiftError`** — UniFFI Error; 2 variants: `SwarmBuild`, `RuntimeSpawn`.

### Orchestrator

**`spawn_for_swift`** — free function annotated `#[uniffi::export(async_runtime = "tokio")]`. Takes `Arc<PeerIdentity>` + `Vec<Multiaddr>` + `Vec<AllowedPeer>` + 3 callback interfaces (listener, heartbeat provider, stream provider). Internally builds the swarm, wires the upstream `PeerLivenessEvent` channel to the Swift listener via a tokio drain task (dropping the 8 other receivers from `NetworkRuntime::spawn`), and returns `Arc<NetworkRuntime>`. Callback-interface params arrive as `Box<dyn ...>` (UniFFI 0.31 Lift convention) and are promoted to `Arc` internally.

### Adapters

- **5 source-stream adapters** (`audio_source_to_stream` through `detection_source_to_stream`): convert `Box<dyn Swift*Source>` → upstream `SourceStream<XFrame>` via a tokio mpsc channel + prost decoding of each `payload_bytes`.
- **`swift_provider_to_upstream`**: wraps `Arc<dyn SwiftStreamProvider>` into the upstream `StreamProvider` closure type, implementing the two-call protocol with prost manifest/reason decoding.

### Upstream-annotated surface (via re-export from `auki-network`)

- **`NetworkRuntime`** — UniFFI Object with: `local_peer_id_string()`, `connected_peer_id_strings()`, `set_allowed_peers(peers)`, `shutdown()`, and 5 async `open_*_stream` methods returning the matching `Arc<StreamSubscription*>`.
- **`StreamSubscriptionAudio`**, **`StreamSubscriptionCamera`**, **`StreamSubscriptionPointCloud`**, **`StreamSubscriptionJointEncoders`**, **`StreamSubscriptionDetection`** — UniFFI Objects; each has `manifest_bytes()` + async `next_entry() -> Result<Option<StreamEntry>, StreamError>`.
- **`StreamEntry`** — UniFFI Record `{ timestamp_ns, seq, payload_bytes }`.
- **`StreamError`**, **`OpenStreamError`** — UniFFI Errors (flattened enums).
- **`AllowedPeer`** — UniFFI Record.
- **Discovery surface** (`DiscoveryClient`, `ClusterEntry`, `CreateClusterOutcome`, `DiscoveryError`) — now upstream-annotated via re-export; Stage 1's hand-wrappers in this crate's `lib.rs` have been deleted.

### Build and validation

- `cargo build -p auki-network-swift --features swift-bindings,swarm,discovery_client` — green.
- `cargo test -p auki-network-swift` — 21 tests green.
- `cargo build --workspace --exclude browser_probe_listener` — green (including `auki-network-py` and `auki-domain` cascading from owned-types change).
- `build-xcframework.sh` — produces `AukiNetwork.xcframework` (device `ios-arm64` + fat simulator `ios-arm64_x86_64-simulator`) cleanly. A pre-existing bug in `WORKSPACE_ROOT` (was 2 levels up instead of 3) was fixed in commit `2530bd8`.

## What does NOT work yet

- **No cluster lifecycle / peer enumeration / `ParticipantInfo`.** That is a separate future `auki-domain-swift`, mirroring the `auki-network-py` → `auki-domain-py` split.
- **Not wired into iosapp.** The `Bridge/` SPM integration is an iosapp-repo task (Spec 2).
- **`with_http` is not exposed.** The Rust `DiscoveryClient::with_http(base_url, reqwest::Client)` escape hatch is deliberately omitted; the Swift binding uses `reqwest::Client::new()` defaults. Tracked in [`parking_lot.md`](../parking_lot.md).
- **Heartbeat-detail events not forwarded.** `HeartbeatReceived` and `HeartbeatNtpSampleObserved` are dropped by the drain task. Tracked in [`parking_lot.md`](../parking_lot.md).

## Rust mapping

| Swift | Rust |
|---|---|
| `DiscoveryClient` | `auki_network::discovery_client::DiscoveryClient` |
| `ClusterEntry` | `auki_network::discovery_client::ClusterEntry` |
| `CreateClusterOutcome` | `auki_network::discovery_client::CreateClusterOutcome` |
| `DiscoveryError` | `auki_network::discovery_client::DiscoveryError` (flattened) |
| `NetworkRuntime` | `auki_network::NetworkRuntime` |
| `AllowedPeer` | `auki_network::AllowedPeer` |
| `StreamSubscription{Audio,...}` | binding-crate Objects wrapping `auki_network::StreamSubscription<T>` |
| `StreamEntry` | binding-crate Record |
| `SwiftPeerLivenessEvent` | 3-variant v0 subset of `auki_network::PeerLivenessEvent` |

## Verification

```bash
cargo build -p auki-network-swift --features swift-bindings,swarm,discovery_client   # green
cargo test  -p auki-network-swift                                                      # 21/21 green
cargo build --workspace --exclude browser_probe_listener                               # green
bindings/swift/auki-network-swift/build-xcframework.sh                                # produces AukiNetwork.xcframework
```
