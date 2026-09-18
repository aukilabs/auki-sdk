# Resolve and connect by identity

`AukiPeer::resolve(identity, protocol_id)` performs a fresh, bounded DDS lookup
in the peer's selected Domain. Enable `DdsTrackerMode::DiscoverOnly` or
`DiscoverAndAdvertise` on the bootstrap first. The remote peer must advertise
its reachable routes and register the requested inbound protocol.

| Identity | DDS query | Connection check |
| --- | --- | --- |
| `AukiPeerIdentity::Peer(id)` | `peer_id` | Exact authenticated transport Peer ID. |
| `AukiPeerIdentity::Robot(id)` | `subject_id`, `peer_type=robot` | Selected Peer ID plus exact signed robot subject and type. |
| `AukiPeerIdentity::Compute(id)` | `subject_id`, `peer_type=compute` | Selected Peer ID plus exact signed compute-node subject and type. |

Each lookup also filters by the exact application protocol. Machine IDs are
typed UUIDs and Peer IDs are typed libp2p identities. A robot or compute node may
advertise several peers. The SDK returns all matching candidates in Peer-ID
order; the application must select a peer rather than assume the first entry is
the machine's only process.

An empty successful result means **no current matching advertisement**. It does
not prove that a machine is offline or missing from DDS inventory. Expired
records and the caller are excluded. Discovery is a live view, so registration
and expiry can change membership while paging.

## Open a selected peer

`AukiResolvedPeer` retains the requested identity, Domain, protocol, and candidate.
Pass it to `AukiPeerProtocols::open_resolved`; do not discard its identity
constraint by extracting a route and calling `open_exact` for a machine lookup.

```rust
use auki_sdk::{
    AukiPeer, AukiPeerIdentity, AuthenticatedRouteStream, PeerId,
};
use uuid::Uuid;

async fn connect_robot_process(
    peer: &AukiPeer,
    robot_id: Uuid,
    selected_peer_id: PeerId,
) -> Result<AuthenticatedRouteStream, Box<dyn std::error::Error>> {
    let candidates = peer
        .resolve(AukiPeerIdentity::Robot(robot_id), "/my-app/control/1.0.0")
        .await?;
    let selected = candidates.iter()
        .find(|result| result.candidate().peer_id() == selected_peer_id)
        .ok_or("selected robot process is not currently advertising this protocol")?;
    Ok(peer.protocols().open_resolved(selected).await?)
}
```

Present `candidate().peer_id()`, `routes()`, and `expires_at()` to help the host
select a process. For an exact Peer ID lookup there can be at most one result.
The cloneable `AukiDiscovery` handle exposes the same `resolve` operation for
Rust adapters that cannot retain a borrow of the peer across an async call.

Opening tries each supported advertised route at most once, in canonical order,
within a 30-second total deadline. Native peers use direct TCP or TCP relay
routes; browser peers use WSS relay routes. Unsupported routes are skipped;
no supported routes, expiry, route failure, and timeout are distinct errors.
Results from another Domain are rejected before dialing. No route is added to
the peer's persistent configured-route list.

The existing mutual P2P verification still checks signature, issuer, audience,
expiry, Peer ID, Domain and required scope. The SDK then compares the signed
remote subject/type against the requested machine before returning application
I/O. A mismatch stops connection attempts and closes the stream, with a bounded
close deadline. `IdentityMismatch.cleanup_failed` reports incomplete explicit
cleanup; dropping the stream also releases its route ownership. Discovery hints
are never authorization evidence.

Set deadlines for your application reads/writes and await `stream.close()` on
success and failure. Dropping the open future cancels the attempt, and peer
shutdown cancels pending lookups and opens. A lookup canceled by peer shutdown
returns `AukiDiscoveryError::Authentication`, consistent with an unavailable
owning authority. Close handlers before awaiting peer shutdown as described in
[peer lifecycle](../how-to/lifecycle.md).

## Completeness and provider rollout

Exact lookups use at most 100 pages of 100 candidates, a 512 KiB response limit
per page, and a 30-second total lookup deadline. Every page must acknowledge the
exact requested filters in `applied_filters`. Missing/mismatched acknowledgement,
wrong identity/protocol metadata, repeated peers or cursors, or exceeded bounds
are errors, never partial success. Ordinary `discover` and `discover_protocol`
remain available for browsing with older providers.

This requires [domain-service #568](https://github.com/aukilabs/domain-service/pull/568)
(provider commit `64166e6c`, based on DDS main `b27c0804`), tracked by
[auki-sdk #386](https://github.com/aukilabs/auki-sdk/issues/386), including migration 45. Roll it out to every
serving DDS replica before enabling exact lookup consumers. Old DDS versions
ignore unknown identity query parameters; the acknowledgement check makes that
incompatibility explicit, even for an empty page. Mixed provider versions can
therefore produce explicit lookup errors. Local source/tests do not prove the
feature is deployed in any shared environment.

These APIs are available in the native and WASM Rust SDK. As with the existing
custom-protocol stream API, language-specific adapters must be compiled into the
same extension/module/framework; this change does not add raw stream methods to
the Python, JavaScript, Swift or Expo bindings. See
[custom protocol adapters](../how-to/protocols.md#call-it-from-python-javascript-or-swift).
It changes no token profile, wire protocol ID, or existing binding method.

Exact P2P connection does not authorize DMS jobs, Domain Server writes, or an
application operation. Keep those admission checks in their owning services.
