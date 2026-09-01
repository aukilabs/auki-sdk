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

Protocol adapters must compile into the same extension module as this facade so
live Rust handles never cross native module boundaries. The
[portable echo Python example](../../../examples/portable-echo/python/README.md)
is the smallest complete host.

`shutdown()` starts one detached ordered Rust cleanup and returns a replayable
awaitable. Cancelling a Python task that waits for shutdown does not cancel DMS
relay deletion or native transport cleanup.
