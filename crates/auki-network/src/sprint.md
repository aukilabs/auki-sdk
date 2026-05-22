# Sprint — auki-network

Current work and next steps to close the gap between [`src/readme.md`](readme.md) (what's implemented) and [the outer README](../README.md) (the spec).

## Now

The crate is the low-level networking substrate. The current implementation has these layers:

- **Core identity and reachability, `default-features = false`.** `PeerIdentity` derives the libp2p key from `Wallet::derive_child("peer/v1")`; `ReachabilityRecord`, `Capability`, and `ParticipantInfo` remain the small serializable shapes available without native transport dependencies. `PeerIdentity::private_key_protobuf()` is the bridge that lets jslibp2p import the same key material in browser JavaScript.
- **Native and browser binding adapters.** The default native feature set enables a small UniFFI surface. The `wasm` feature exposes identity/protocol helpers to wasm-bindgen. Crate-owned JavaScript templates generate a package whose runtime imports jslibp2p lazily; browser transport is not implemented by Rust `NetworkRuntime`.
- **Browser probe, `browser_probe` feature.** The crate exposes shared `/auki/browser-probe/0.0.1` request/response structs and a native WebRTC Direct listener example that prints browser-dialable `/webrtc-direct/certhash/.../p2p/<peer-id>` multiaddrs. This is a proof surface for browser peers only; production Domain join still lives above it.
- **Native message node, `message_node` feature.** The crate now owns a small synchronous WebRTC Direct `/auki/message/0.0.1` facade for UniFFI hosts. It uses generated `auki-proto` message bytes at the boundary, keeps protobuf conversion platform-local, exposes `AukiMessageNode` in generated Swift, backs the iOS generated-binding test app, and proves native-to-native message/ack exchange with a local WebRTC Direct test.
- **Swarm and protocols, `swarm` feature.** `swarm::build_swarm` builds TCP + QUIC + Noise + Yamux with identify, ping, relay-client, optional relay-server, and the raw-substream behaviour. `NetworkRuntime` owns the swarm task, dynamic allowed peers, connected-peer snapshots, membership broadcast, join requests, peer-info requests, sensor-catalog requests, registry-entry requests, typed stream opening, and shutdown.
- **Discovery client, `discovery_client` feature.** `DiscoveryClient` wraps Discovery's cluster directory endpoints: list, atomic create, liveness check, Manager rotation, and deregistration.

Current libp2p protocol modules:

| Protocol | Module | Current role |
|---|---|---|
| `/auki/join/0.0.1` | `join_protocol` | Non-member asks the Manager for admission |
| `/auki/heartbeat/0.0.1` | `heartbeat_protocol` | Pairwise liveness used for Manager-death detection |
| `/auki/membership/0.0.1` | `membership_protocol` | Manager broadcasts fresh membership JSON |
| `/auki/info/0.0.1` | `info_protocol` | Fetch another peer's `ParticipantInfo` |
| `/auki/browser-probe/0.0.1` | `browser_probe_protocol` / `browser_probe` | Native WebRTC Direct request/response probe for browser peers |
| `/auki/message/0.0.1` | `message_protocol` / `message_node` | Generic protobuf envelope/ack exchange for generated binding clients |
| `/auki/sensors/0.0.1` | `sensors_protocol` | Fetch another peer's current sensor catalog, optionally with Sensor / Frame Registry JSON embedded by value |
| `/auki/registries/0.0.1` | `registries_protocol` | Fetch exact Sensor / Clock / Frame Registry entries by `(kind, id, hash)` |
| `/auki/stream/0.1.0` | `stream_protocol` / `stream_runtime` | Typed live sensor streams |

The typed stream runtime is multi-payload on both sides. Producer callbacks have the requester peer id in their signature:

```rust
type StreamProvider =
    Arc<dyn Fn(PeerId, StreamRequest) -> StreamDispatch + Send + Sync>;
```

`StreamDispatch` supports JPEG, PointCloud, JointEncoders, Audio, and decline paths. Each accepted substream is still mono-`T`; the consumer chooses the expected payload type when opening the stream.

Cluster lifecycle policy is intentionally one layer up in [`auki-domain`](../../auki-domain). App daemons should normally use `ClusterManager`; this crate remains the transport/protocol toolbox.

## Next

In priority order:

1. **Live iOS/browser interop run.** The generated Swift host and browser smoke script exist. Run the app on a simulator or device, copy a live `/webrtc-direct/.../p2p/<peer-id>` listen address, and verify the browser js-libp2p peer can send `/auki/message/0.0.1` `MessageEnvelope` bytes to the Rust-backed iOS node.

2. **Capability and topic discovery.** The architecture still needs a peer-to-peer way to advertise current capabilities and available sensor topics. The likely shape is a request/response protocol sibling to the current info and sensors protocols, owned by `NetworkRuntime` and surfaced through `auki-domain`.

3. **Protocol hardening.** Successor-token verification, challenge/response, and tighter Discovery trust checks belong with the next security pass. Discovery v1 is shape-checked, not signature-verified.

4. **Transport reachability upgrades.** DCUtR / hole-punching and AutoNAT are additive improvements once real deployments need better direct-connect behavior.

## Smaller Follow-Ups

- Expose only the `SwarmConfig` knobs that real daemons need; keep idle/ping/connection-limit tuning private until there is pressure.
- Decide whether transport build errors should stay stringly typed or preserve structured sources.
- Keep the Python stream-provider bridge aligned when new `StreamDispatch` payload variants land.
- Keep the generated JavaScript package at the network layer; add a separate facade only if an app needs one after browser leaf-peer join works.

## Open Items

See [`parking_lot.md`](../parking_lot.md). Remaining items are forward-looking; none block the current `ClusterManager` path.
