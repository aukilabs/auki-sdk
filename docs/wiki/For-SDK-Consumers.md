# For SDK Consumers

Use the smallest SDK owner that matches the job. Local recording and
authenticated networking are independent; applications compose them only when
they need both.

| Goal | Start here |
| --- | --- |
| Start an authenticated Rust or Web peer | [Auki P2P](https://github.com/aukilabs/auki-sdk/blob/develop/docs/p2p/README.md) |
| Use an existing protocol | [P2P getting started](https://github.com/aukilabs/auki-sdk/blob/develop/docs/p2p/getting-started.md) |
| Author one portable protocol | [Protocol authoring](https://github.com/aukilabs/auki-sdk/blob/develop/docs/p2p/authoring-protocols.md) |
| Record registries, manifests, and logs | [Quickstart](Quickstart) |
| Understand source and writer identity | [Concept: peer-owned logs](Concept-Peer-Owned-Logs) |

## Networking

`auki_sdk::AukiPeer` is the networking facade. It owns one authenticated Peer
ID in one selected DDS Domain, credential renewal, transport, relay booking,
routes, protocol registration, fencing, and shutdown.

An `AukiPeer` serves no application protocol by default. Enable only the
`auki-protocols` families you need, then explicitly mount their `Endpoint`.
Use the matching cloneable `Client` for outbound operations. Applications
still provide the remote Peer ID and route; relay allocation is reachability,
not discovery.

The live resource service is Catalog v3. Its ordinary sensor, pose,
time-transform, and detection rows retain the Catalog v2 JSON shape, while v3
also adds message-channel rows. Catalog v2 remains a wire-compatibility schema,
not a mounted endpoint. Map Logs use Catalog v4.

## Local recording

`auki_session::Peer` owns durable local registries. A `Session` is one recording
timeline with clocks and log handles. Neither type starts networking.

Native Rust can project one `Peer`/`Session` pair into the Catalog and Stream
endpoints with `auki_protocols::session_adapter::SessionProtocolProvider`.
That adapter is mechanical; the application still decides which authenticated
requesters may see or subscribe to its data.

## Platform status

| Platform | Authenticated peer facade |
| --- | --- |
| Native Rust | Available: User/App auth, persistent identity, relay, endpoints, shutdown |
| Web/Wasm | Available: User auth, ephemeral identity, mandatory WSS relay, endpoints, shutdown |
| Python | Pending; component-level identity/session/log/registry bindings remain available |
| Swift/iOS | Pending; the current Swift binding covers `Wallet` only |

Do not start a new integration on the removed Manager or `Domain` runtime.
Historical behavior remains documented in [Release history](Release-History)
and old tags.

## Keep these boundaries

- A route says where to dial; it grants no authority.
- An authenticated peer shares a Domain; product policy still decides what it
  may do.
- A protocol ID names one immutable wire contract.
- Close mounted endpoints before awaiting `peer.shutdown()`.
- Use one live runtime per persisted Peer ID.

For version-specific changes, consult [Release history](Release-History) and
the exact tag or Git revision your application pins.
