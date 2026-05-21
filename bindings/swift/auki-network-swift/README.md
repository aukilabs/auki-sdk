# auki-network-swift

UniFFI Swift bindings for the full v0 networking surface of [`auki-network`](../../../crates/auki-network).

This crate is the Swift sibling of [`auki-network-py`](../../python/auki-network-py): one binding crate per Rust component, no umbrella `auki-swift`. It mirrors that crate's split — Discovery + runtime + stream surface here; cluster lifecycle / peer enumeration will arrive as a future `auki-domain-swift`, paired with [`auki-domain`](../../../crates/auki-domain) the same way [`auki-domain-py`](../../python/auki-domain-py) pairs with it on the Python side.

The consumer is [`aukilabs/iosapp`](https://github.com/aukilabs/iosapp), a native iOS Auki peer. Its Q1 decision (UniFFI, thin transport binding, prost payloads decoded Swift-side via swift-protobuf) is the spec this crate implements.

Wallet and PeerIdentity are provided by [`auki-identity-swift`](../auki-identity-swift) (PR A — already shipped).

## Surface

### Discovery

```swift
let client = DiscoveryClient(baseUrl: "http://discovery.lan:8080")

let clusters = try await client.listClusters()
let outcome  = try await client.createCluster(
    name: "demo",
    managerPeerId: peerId,
    managerMultiaddrs: ["/ip4/192.168.9.72/tcp/4001"])

let entry = try await client.livenessCheck(name: "demo", peerCount: 2)
_         = try await client.rotateManager(name: "demo",
                                           managerPeerId: peerId,
                                           managerMultiaddrs: addrs)
try await client.deregister(name: "demo")
```

### Runtime + spawn

```swift
let runtime = try await spawnForSwift(
    identity: identity,            // Arc<PeerIdentity> from auki-identity-swift
    listenAddrs: ["/ip4/0.0.0.0/tcp/0"],
    allowedPeers: [],
    livenessListener: myListener,  // PeerLivenessListener callback interface
    heartbeatProvider: myHbProvider, // HeartbeatTimestampProvider callback interface
    streamProvider: myStreamProvider // SwiftStreamProvider callback interface
)

let localId  = runtime.localPeerIdString()
let peerIds  = runtime.connectedPeerIdStrings()
runtime.setAllowedPeers(peers: updatedPeers)
runtime.shutdown()
```

### Stream consumer (open side)

```swift
let sub = try await runtime.openAudioStream(peerId: id, requestBytes: reqBytes)
let manifest = sub.manifestBytes()
while let entry = try await sub.nextEntry() {
    // entry.timestampNs, entry.seq, entry.payloadBytes (prost-encoded AudioFrame)
}
```

One `open*Stream` method per payload type: `openAudioStream`, `openCameraStream`, `openPointCloudStream`, `openJointEncodersStream`, `openDetectionStream`.

### Stream producer (provide side) — two-call protocol

Swift implements `SwiftStreamProvider`:

1. `dispatchDecision(peerId:requestBytes:) -> SwiftStreamDecision` — return `.acceptAudio(manifestBytes:)` / `.acceptCamera(...)` / ... / `.decline(reasonBytes:)`.
2. If an Accept variant was returned, the runtime calls the matching `*Source(peerId:requestBytes:) -> Box<dyn Swift*Source>` method to obtain the frame source.

This two-call split is a UniFFI 0.31 constraint: trait objects cannot be fields inside `uniffi::Enum` variants.

Each `Swift*Source` trait exposes a single `nextItem() -> Result<StreamItem?, SwiftSourceError>` method. `StreamItem` carries `{ timestampNs: Int64, payloadBytes: Data }`.

## Types

| Swift type | Rust source |
|---|---|
| `DiscoveryClient` | `auki_network::discovery_client::DiscoveryClient` |
| `ClusterEntry` | `auki_network::discovery_client::ClusterEntry` |
| `CreateClusterOutcome` | `auki_network::discovery_client::CreateClusterOutcome` |
| `DiscoveryError` | `auki_network::discovery_client::DiscoveryError` (flattened) |
| `NetworkRuntime` | `auki_network::NetworkRuntime` |
| `AllowedPeer` | `auki_network::AllowedPeer` |
| `StreamSubscriptionAudio` | binding-crate Object wrapping `StreamSubscription<AudioFrame>` |
| `StreamSubscriptionCamera` | binding-crate Object wrapping `StreamSubscription<CameraFrame>` |
| `StreamSubscriptionPointCloud` | binding-crate Object wrapping `StreamSubscription<PointCloudFrame>` |
| `StreamSubscriptionJointEncoders` | binding-crate Object wrapping `StreamSubscription<JointEncodersFrame>` |
| `StreamSubscriptionDetection` | binding-crate Object wrapping `StreamSubscription<DetectionFrame>` |
| `StreamEntry` | `{ timestampNs, seq, payloadBytes }` record |
| `StreamError` | flattened stream consumer error |
| `OpenStreamError` | flattened open-stream error |
| `SwiftPeerLivenessEvent` | 3-variant v0 enum (Connected, Disconnected, HeartbeatAlive) |
| `SpawnSwiftError` | `{ SwarmBuild, RuntimeSpawn }` |

Callback-interface traits: `PeerLivenessListener`, `HeartbeatTimestampProvider`, `SwiftStreamProvider`, `SwiftAudioSource`, `SwiftCameraSource`, `SwiftPointCloudSource`, `SwiftJointEncodersSource`, `SwiftDetectionSource`.

`PeerId` and `Multiaddr` cross the FFI as canonical strings via `uniffi::custom_type!` with the `remote` keyword (required for foreign-crate types under UniFFI 0.31's orphan rules). Wire payloads are opaque `Data`; Swift decodes via swift-protobuf against the committed `.proto` files in `crates/auki-datatypes/proto/`.

## iOS targets

The `build-xcframework.sh` script produces `AukiNetwork.xcframework` with two slices:

- `ios-arm64` — physical device
- `ios-arm64_x86_64-simulator` — fat simulator (Apple Silicon + Intel)

TLS: `reqwest 0.12` → `rustls 0.23` → `ring 0.17` (no `aws-lc-rs`). `ring 0.17.x` has first-class iOS cross-compile support; no `CC`/SDK env intervention required.

## Build

Host check (the in-workspace gate):

```bash
cargo build -p auki-network-swift --features swift-bindings,swarm,discovery_client
cargo test  -p auki-network-swift
```

iOS XCFramework:

```bash
bindings/swift/auki-network-swift/build-xcframework.sh
```

## Status

PR B complete. See [`src/readme.md`](src/readme.md) for what is implemented and [`src/sprint.md`](src/sprint.md) for what is next (PR C: `auki-domain-swift`).
