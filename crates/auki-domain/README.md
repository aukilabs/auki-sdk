# auki-domain

The SDK's authenticated Domain facade. One `Domain` owns one stable
`auki-p2p` identity and joins one exact DDS Domain UUID. It exposes the Auki
application protocols retained by the SDK without recreating the former
Manager, cluster-membership, election, or Discovery control plane.

## Ownership boundary

The host application owns credential acquisition. It fetches DDS verification
keys and a signed P2P Domain credential, then supplies both to
`DomainBuilder::authority(...)`. `auki-domain` verifies and refreshes the
runtime authority supplied by the host; it does not make DDS or DMS HTTP
requests itself.

The host also supplies listeners and exact-peer direct or complete relay-circuit
dial hints. `auki-p2p` owns dialing, secure transport, admission, and
authentication over those routes. Separately, lower-level hosts given an
authorized `RelayProvider` assignment can use `auki-p2p::Node` reservation
mechanics. Provider booking/HTTP and confirmed-route distribution stay outside
the SDK. Application protocols are never exposed on an unauthenticated
connection.

## Joining a Domain

- `DomainConfig::new(domain_id, identity)` selects the exact DDS Domain UUID
  and stable P2P identity.
- `DomainConfig::with_listen_addresses(...)` configures local listeners. Zero
  listeners is valid.
- `DomainConfig::with_peer_routes(peer, routes)` installs initial exact-peer
  route candidates.
- `Domain::builder(&peer, &session, config)` composes the existing
  `auki-session` peer/session data with the authenticated transport.
- `DomainBuilder::authority(keys, credential)` is required before `join()`.
- `DomainBuilder::served_protocols(ServedProtocols::none()...)` explicitly
  selects the exact inbound protocol versions. Omitting it serves none; client
  operations remain compiled and available.
- `DomainBuilder::participant_info_provider(...)`,
  `resource_catalog_provider(...)`, `map_catalog_provider(...)`,
  `registry_app_root(...)`, `stream_provider(...)`, and `message_channel(...)`
  install application-owned providers and bounded receivers before protocol
  handlers start.
- `DomainBuilder::join()` binds listeners, verifies the identity/domain chain,
  installs only the selected services, and returns once the Domain is ready.
- `Domain::leave()` performs bounded, ordered shutdown. Dropping the Domain is
  the best-effort backstop and still fences cloned handles immediately.

`Peer`, `Session`, the configured `Identity`, and the signed credential must all
identify the same peer and Domain. A mismatch fails before application protocol
traffic is accepted.

## Public lifecycle and transport views

- `status()` / `subscribe_status()` expose lifecycle state and terminal failure.
- `authority()` installs refreshed verification keys or a replacement signed
  credential supplied by the host.
- `routes()` replaces, removes, or snapshots explicit route candidates.
- `known_peers()` snapshots and subscribes to peers that are both authenticated
  for this Domain and currently reachable. It is not a membership roster.
- `protocols()` registers and opens additional authenticated protocols without
  exposing the underlying node.
- `served_protocol_ids()` reports the exact built-in IDs selected for this
  Domain instance; the list is diagnostic, not remote authority.
- `domain_id()`, `peer_id()`, and `listen_addresses()` expose stable identity
  and bound listener state.

## Retained Auki protocols

The facade can serve the useful product operations while changing their
transport and authorization internals. Each exact version is opt-in through
`ServedProtocols`:

- participant info;
- resource catalogs v0.2, v0.3, and v0.4;
- hash-pinned registry entries and bounded blob reads;
- typed streams and map streams;
- bounded, receiver-owned message channels;
- registration of additional authenticated protocols.

The `catalog()`, `catalog_of(...)`, and `map_catalog_of(...)` helpers still
derive catalog rows from `Peer` and `Session`. Registry and blob serving is
rooted at the peer's configured storage directory. Message receiver ownership
continues to control channel lifetime.

Every remote operation targets an expected `PeerId`. Route fallback is allowed
only among candidates for that exact peer, and the authenticated remote identity
must match before a response is accepted. Local-self operations use the same
service state without opening a network connection.

## Depends on

- [`auki-p2p`](../auki-p2p) — identity, authenticated transport, admission,
  dialing, and relay-reservation primitives.
- [`auki-protocols`](../auki-protocols) — pure wire payloads, framing, IDs, and
  validation. Provider/subscription types stay here with the hosting runtime.
- [`auki-session`](../auki-session) — application peer/session data and catalog
  sources.
- [`auki-registry`](../auki-registry) and [`auki-hash`](../auki-hash) —
  hash-pinned registry and blob operations.

The old `ClusterManager`, membership document, join/election/heartbeat
protocols, Discovery client, browser session shim, and Domain-time authority are
not part of this crate.

See the [authenticated Domain migration guide](../../docs/authenticated-domain-migration.md)
for the Rust/Python breaking-change table, route-source boundary, relay flow,
and runnable two-peer diagnostic example.
