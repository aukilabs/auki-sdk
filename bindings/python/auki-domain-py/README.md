# auki-domain-py

PyO3 bindings for [`auki-domain`](../../../crates/auki-domain). The module owns
one authenticated Auki Domain node and exposes its application protocols to
Python.

The host application supplies:

- a stable libp2p `Identity`;
- a DDS Domain UUID;
- DDS ES256 verification keys and a signed P2P credential for that identity;
- listen addresses and explicit routes to peers, when needed; and
- application providers such as the resource catalog.

Credential acquisition is intentionally outside this package. The binding does
not make hidden DDS HTTP requests. It verifies the supplied credential and
fails closed when its peer identity, Domain UUID, signature, or expiry is
invalid.

## Build locally

From the repository root:

```sh
uv venv --python 3.12 .venv
source .venv/bin/activate
uv pip install maturin pytest
maturin develop -m bindings/python/auki-session-py/Cargo.toml
maturin develop -m bindings/python/auki-domain-py/Cargo.toml \
  --features test-support
python -m pytest bindings/python/auki-domain-py/python_tests
```

The authenticated tests use the non-default `test-support` feature to create
deterministic credentials. It embeds test-only signing material and must never
be enabled in a release wheel. Build a normal local/release extension without
that feature:

```sh
maturin develop -m bindings/python/auki-domain-py/Cargo.toml
```

## Join an authenticated Domain

`auki_session.Peer` owns the process peer and Session. `auki_domain.Domain`
borrows both through their capsule bridge and keeps the Rust objects alive for
the joined node.

```python
import asyncio
from pathlib import Path

import auki_domain
import auki_session


async def main() -> None:
    # Persist these bytes and reuse them on restart. Generating a new identity
    # produces a new PeerId and therefore requires a new signed credential.
    identity = auki_domain.Identity.from_protobuf_encoding(
        Path("robot-p2p-identity.protobuf").read_bytes()
    )

    peer = auki_session.Peer(identity.peer_id, "robot")
    peer = peer.with_storage_root("./robot-session-data")
    session = peer.start_session()

    config = auki_domain.DomainConfig(
        "de66fdf4-a830-4017-95dd-5741c30a6d0f",
        identity,
    )
    config.with_listen_addresses(["/ip4/0.0.0.0/tcp/0"])

    keys = auki_domain.DdsVerificationKeys(
        7,
        Path("dds-p2p-current-es256-public.pem").read_bytes(),
        None,
    )
    credential = auki_domain.SignedP2pCredential(
        Path("dds-p2p-credential.jwt").read_text().strip()
    )

    builder = auki_domain.Domain.builder(peer, session, config)
    # Direct construction is equivalent:
    # builder = auki_domain.DomainBuilder(peer, session, config)
    builder.authority(keys, credential)
    builder.serve_info_v1()
    builder.serve_resources_v2()
    domain = await builder.join()

    try:
        print(domain.peer_id, domain.domain_id, domain.listen_addresses)
        print(domain.status().state)  # "ready"
    finally:
        await domain.leave()


asyncio.run(main())
```

Builder setters mutate the builder and return `None`; call them before
`await builder.join()`. A builder is single-use. `Domain.leave()` is idempotent
and should be awaited for deterministic shutdown.

`DomainBuilder` serves no built-in application protocols by default. Select
only exact inbound versions this application hosts before joining:
`serve_info_v1`, `serve_resources_v2`, `serve_resources_v3`,
`serve_resources_v4`, `serve_registries_v2`, `serve_registries_v3`,
`serve_blobs_v1`, `serve_messages_v1`, and `serve_streams_v2`. Client methods
remain available without the matching inbound selection.
`domain.served_protocol_ids` reports the selected exact IDs.

A wallet-backed host derives the same canonical identity by passing
`wallet.derive_child("peer/v1").seed()` from `auki-identity-py` to
`Identity.from_ed25519_seed(...)`. Persist the wallet seed or the canonical
protobuf identity bytes; never silently generate a replacement for invalid
state because the resulting PeerId will not match its credential.

## Routes and authenticated peers

Routes are explicit expected-PeerId-to-multiaddr hints. The remote peer still
has to prove a valid credential for the same DDS Domain before any application
protocol is served.

```python
routes = domain.routes()
routes.replace(
    remote_peer_id,
    ["/dns4/relay.example.com/tcp/443/p2p/RELAY/p2p-circuit/p2p/REMOTE"],
)

rows = await domain.fetch_resources_catalog(remote_peer_id)
print([row.resource_id for row in rows])

for peer in domain.known_peers().snapshot():
    print(peer.peer_id, peer.authenticated_until)
```

`known_peers()` contains peers that are currently transport-connected with
unexpired same-Domain authority after completing an authenticated application
stream. A peer disappears on its last connection close or credential expiry.
This is not a configured-route list or discovery source;
`domain.routes().snapshot()` returns the current dial hints.

Verification keys and the local credential can be rotated without replacing
the node:

```python
authority = domain.authority()
await authority.install_verification_keys(next_keys)
await authority.install_credential(next_credential)
```

## Resource Catalog provider

`ResourceEntry` mirrors the authenticated
`/auki/auth/1/resources/0.2.0` wire row. Construct rows from a Python dict or a
JSON string and register a callable that returns the catalog's current state.

```python
ZERO_HASH = "0" * 32

camera = auki_domain.ResourceEntry.from_dict({
    "variant": "sensor_log",
    "source_peer_id": identity.peer_id,
    "writer_peer_id": identity.peer_id,
    "resource_id": "head_left_rgb",
    "state": "live",
    "head": {"kind": "rolling", "retention_ns": 5_000_000_000},
    "available": {"bytes": 0, "entries": 0, "duration_ns": 0},
    "sensor": {
        "kind": "camera",
        "type": "rgb",
        "sensor_id": "head_left_rgb",
        "sensor_hash": "camera-registry-hash",
    },
    "manifest": {
        "clock": {
            "peer_id": identity.peer_id,
            "id": "session/sdk_clock",
            "hash": ZERO_HASH,
        },
        "frame": None,
    },
})

# Preferred when the provider is available before joining:
builder.resource_catalog_provider(lambda: [camera])
domain = await builder.join()

# It can also be installed or replaced after joining:
domain.set_resource_catalog_provider(lambda: [camera])
```

The callback is invoked for each authenticated inbound fetch. Return only rows
that can currently accept stream opens. Provider exceptions and values that
are not `ResourceEntry` instances are logged and sampled as an empty catalog;
they are never partially converted.

All four v0.2 variants are supported: `sensor_log`, `pose_log`,
`time_transform_log`, and `detection_log`. Invalid dicts and JSON raise
`ValueError` with the underlying schema error.

## Live participant and stream providers

Participant metadata can be sampled on every authenticated info request. This
keeps values such as `session_now_ns` live instead of freezing them at join:

```python
builder.participant_info_provider(lambda: auki_domain.ParticipantInfo(
    "robot-app", "1.0.0", "robot", session.session_id,
    "session-clock", clock_hash, session_now_ns(), identity.peer_id,
))
```

Python can also publish every retained typed stream through the same owned
Domain. The callback is synchronous; its source is an async iterator which
runs on the caller's captured asyncio loop:

```python
async def camera_frames():
    try:
        while True:
            frame, timestamp_ns = await next_frame()
            yield auki_domain.StreamItem(
                timestamp_ns=timestamp_ns,
                payload=auki_domain.CameraFrame(frame),
            )
    finally:
        await close_camera()

def streams(requester_peer_id, request):
    if request.resource_id != "head-camera":
        return auki_domain.StreamDecision.decline(
            auki_domain.DeclineReason.sensor_not_found()
        )
    manifest = auki_domain.StreamManifest(
        sensor_id="head-camera",
        sensor_hash=sensor_hash,
        clock_id="session-clock",
        clock_hash=clock_hash,
    )
    return auki_domain.StreamDecision.accept_camera(
        manifest=manifest,
        source=camera_frames(),
    )

builder.stream_provider(streams)
```

Dropping a consumer, cancelling an operation, or leaving the Domain cancels
and closes active async sources. `Domain.leave()` waits for those bounded
finalizers. `StreamDecision.accept_source(auki_logs.StreamSource)` preserves
the retained-log publishing path without a cross-extension Rust capsule.

## Main Python types

| Python class | Role |
|---|---|
| `Identity` | Stable Ed25519 libp2p identity and PeerId |
| `DomainConfig` | DDS Domain UUID, identity, listen addresses, and initial routes |
| `DdsVerificationKeys` | Versioned current and optional previous DDS ES256 public keys |
| `SignedP2pCredential` | Compact DDS-signed credential bound to Domain and PeerId |
| `DomainBuilder` | Pre-join authority and application-provider configuration |
| `Domain` | Joined node and application-protocol entry point |
| `DomainAuthority` | Runtime key and credential rotation |
| `DomainRoutes` | Explicit route replacement, removal, and snapshots |
| `KnownPeers` | Snapshot and subscription APIs for authenticated peers |
| `ResourceEntry` | Resource Catalog v0.2 row |
| `MapLogResource` | Map Catalog row |
| `MessageChannelResource` | Exact receiver identity for a live message channel |
| `ReadFrom` / `StreamRequest` | Stream start position and open request |
| `ParticipantInfo` | Authenticated application identity metadata |

The `Domain` object also exposes resource, map, registry, blob, stream, and
message-channel operations. Network and lifecycle operations return Python
awaitables. Provider setters and local route mutations are synchronous.

## Breaking migration from the Manager-era package

| Removed surface | Stage 1 replacement |
|---|---|
| `ClusterManager`, `ClusterTarget`, membership, election, and Manager roles | `DomainBuilder` and one owned `Domain` |
| Discovery-owned startup and synchronized topology | host-supplied `DomainRoutes`; `KnownPeers` reports mutually authenticated traffic only |
| hidden DDS registration/token HTTP | host-fetched `DdsVerificationKeys` and `SignedP2pCredential` |
| heartbeat-derived shared Domain time | application Session clocks and explicit timestamps |
| `shutdown()` and implicit object teardown | bounded, awaitable `leave()`; cancellation and GC trigger the same native cleanup owner |
| the prior `auki-network-py` runtime bridge | protocol types and producer/consumer stream APIs in `auki-domain-py` |

`auki-domain-py==0.1.0` requires exactly `auki-session-py==0.1.0`. Their
private Peer/Session capsule bridge contains Rust `Arc<T>` values and therefore
must be built atomically from the same SDK commit, `Cargo.lock`, Rust toolchain,
target, feature set, and allocator. The capsule ABI name is revised whenever
that build contract changes; a mismatched wheel is rejected before its payload
is read.

Generic `Domain::protocols()` authoring remains a Rust-only extension surface
in Stage 1. Python applications receive the bounded catalog, registry, blob,
message, and typed-stream clients above; a future generic Python protocol
author API must own its handler tasks and expose bounded byte reads rather than
an unsafe raw-stream shortcut.

## Depends on

- [`auki-domain`](../../../crates/auki-domain) for authenticated lifecycle and
  protocol ownership;
- [`auki-session`](../../../crates/auki-session) and
  [`auki-session-py`](../auki-session-py) for the shared process peer and
  Session; and
- [`auki-protocols`](../../../crates/auki-protocols) for application wire
  contracts.
