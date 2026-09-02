# auki-sdk-py

Thin async Python bindings for the canonical native Rust `AukiPeer` runtime.

```python
session = await AukiSession.login_dev(email, password)
domains = await session.accessible_domains()
peer = await session.start_peer(domains[0].id, "./state/peer.identity")
try:
    routes = peer.routes
    print(peer.peer_id, routes.tcp, routes.wss)
finally:
    await peer.shutdown()
```

`AukiSession.login_app_dev(access_key, secret)` provides the trusted native App
flow. Domain choice remains explicit, identities are persisted by Rust, and a
public relay is required by default. The binding does not expose credentials,
raw transport streams, relay booking, or a Python-owned Tokio runtime.
`peer.routes` is one atomic snapshot: `tcp` and `wss` are both required routes
from the same relay-provider slot and reservation.

DDS discovery is opt-in through the optional `discovery_mode` argument:

```python
peer = await session.start_peer(
    domains[0].id,
    "./state/peer.identity",
    discovery_mode="discover_only",  # or "discover_and_advertise"
)
candidates = await peer.discover_protocol("/example/echo/1.0.0")
```

The candidates are fresh, untrusted route hints. Exact protocol dialing still
verifies the expected Peer ID and Domain in Rust.

Protocol adapters must compile into the same extension module as this facade so
live Rust handles never cross native module boundaries. The
[portable echo Python example](../../../examples/portable-echo/python/README.md)
is the smallest complete host.

## Built-in protocol bindings

The default `standard-protocols` feature enables the complete built-in surface:

| Feature | Python surface |
| --- | --- |
| `info` | `AukiInfoClient` and provider-backed `AukiInfoEndpoint` |
| `catalog` | `AukiCatalogClient` and provider-backed `AukiCatalogEndpoint` |
| `registry` | `AukiRegistryClient` and provider-backed `AukiRegistryEndpoint` |
| `blob` | `AukiBlobClient` and provider-backed `AukiBlobEndpoint` |
| `message` | client, endpoint, sender, and receiver lifecycle |
| `stream` | client, producer-backed endpoint, and consumer subscription |
| `finite-protocols` | Info, Catalog, Registry, and Blob |
| `standard-protocols` | all six families; enabled by default |

Embedders can disable default features, enable only the needed families, and
call `register_facade`, `register_protocols`, or `register_sdk` from their
same-module PyO3 extension.

## Provider and lifecycle contract

- Provider callbacks receive verified requester metadata, never credentials or
  authentication proofs.
- Info, Catalog, Registry, and Stream admission providers are synchronous.
  Rust may call them from runtime worker threads, so keep them fast,
  thread-safe, and free of event-loop assumptions.
- Blob providers may return immediately or return an awaitable. Mount Blob and
  Stream endpoints from their owning running asyncio loop.
- A Stream provider returns an admission result whose accepted form contains a
  payload kind, manifest, and Python async iterable source.
- Await endpoint `close()`, sender/receiver `close()`, and subscription
  `cancel()` before `peer.shutdown()`. These barriers stop admission and finish
  or cancel owned work; dropping handles only starts best-effort cleanup.

`shutdown()` starts one detached ordered Rust cleanup and returns a replayable
awaitable. Cancelling a Python task that waits for shutdown does not cancel DMS
relay deletion or native transport cleanup.
