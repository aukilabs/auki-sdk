# auki-network

Transport-neutral application protocol codecs and plain networking types for
the Auki SDK.

`auki-network` no longer owns a libp2p swarm, discovery client, Manager
lifecycle, membership control plane, heartbeat loop, relay reservation, or
protocol task runtime. `auki-p2p` owns authenticated transport and
`auki-domain` owns Domain policy and application protocol tasks.

## Public surface

The default feature set provides:

- canonical `auki_p2p::Identity` re-export and wallet derivation through
  `identity_from_wallet`;
- the deprecated one-release `PeerIdentity` compatibility adapter;
- plain `ReachabilityRecord` and `Capability` values;
- authenticated application protocol IDs in `protocol_ids`.

The `protocol-codecs` feature adds:

- framing and payload codecs for info, resources v0.2/v0.3/v0.4, registries,
  blobs, typed messages, and typed streams;
- codec-level business validation and bounded frame sizes;
- transport-neutral stream provider, dispatch, entry, subscription, and error
  types in `stream_runtime`;
- `MapCatalogProvider`, used by the Domain map-catalog handler.

The `app_instance` feature adds the platform-specific application-instance
identifier helper. The `swift-bindings` feature retains the identity adapter's
UniFFI surface for the compatibility window.

Protocol selection belongs to the authenticated runtime. Codec modules do not
publish the retired unauthenticated `/auki/...` protocol constants. Use the
`/auki/auth/1/...` constants from `protocol_ids`.

## Locked fixtures

Canonical JSON fixtures pin catalog rows and stream-request shapes across
language implementations:

```sh
cargo test -p auki-network --no-default-features --features protocol-codecs --test locked_json
cargo run -p auki-network --bin regen_locked_fixtures --features protocol-codecs
```

## Dependencies

- [`auki-p2p`](../auki-p2p) — canonical P2P identity and authenticated protocol
  identifiers.
- [`auki-identity`](../auki-identity) — wallet child derivation.
- [`auki-datatypes`](../auki-datatypes) — typed protocol payloads.
- [`auki-manifests`](../auki-manifests) and
  [`auki-registry`](../auki-registry) — catalog and content-addressed reference
  types.
