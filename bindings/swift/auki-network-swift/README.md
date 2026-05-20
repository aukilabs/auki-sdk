# auki-network-swift

UniFFI Swift bindings for the iOS-facing pieces of [`auki-network`](../../../crates/auki-network).

This crate is the Swift sibling of [`auki-network-py`](../../python/auki-network-py): one binding crate per Rust component, no umbrella `auki-swift`. It mirrors that crate's split — Discovery + stream types here; cluster lifecycle / peer enumeration is a future `auki-domain-swift`, paired with [`auki-domain`](../../../crates/auki-domain) the same way [`auki-domain-py`](../../python/auki-domain-py) pairs with it on the Python side.

The consumer is [`aukilabs/iosapp`](https://github.com/aukilabs/iosapp), a native iOS Auki peer. Its Q1 decision (UniFFI, thin transport binding, prost payloads decoded Swift-side via swift-protobuf) is the spec this crate implements.

## Surface (target)

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

| Swift type | Rust source |
|---|---|
| `DiscoveryClient` | `auki_network::discovery_client::DiscoveryClient` |
| `ClusterEntry` | `auki_network::discovery_client::ClusterEntry` |
| `CreateClusterOutcome` | `auki_network::discovery_client::CreateClusterOutcome` |
| `DiscoveryError` | `auki_network::discovery_client::DiscoveryError` (flattened) |

Methods are **async** in Swift (unlike `auki-network-py`'s sync shape — iOS callers must not block the main thread on network I/O). UniFFI drives the futures on a tokio runtime. `PeerId`/`Multiaddr` cross the FFI as canonical strings; errors are a flat enum Swift can switch on.

## Build

Host check (the in-workspace gate):

```bash
cargo build -p auki-network-swift
cargo test  -p auki-network-swift
```

iOS XCFramework (scaffolded — see [`build-xcframework.sh`](build-xcframework.sh) and `src/sprint.md` for what's verified vs. pending):

```bash
crates/auki-network-swift/build-xcframework.sh
```

## Status

Stage 1. See [`src/readme.md`](src/readme.md) for what actually works today and [`src/sprint.md`](src/sprint.md) for what's next (Stage 2 audio streams, Stage 3 `auki-domain-swift`).
