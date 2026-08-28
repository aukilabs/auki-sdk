# auki-sdk

`auki-sdk` is the mechanical runtime facade for authenticated Auki peers. An
`AukiPeer` composes one stable identity and `PreparedPeer` with Domain
participation, SDK `Peer` and `Session` data, configured routes, credential
renewal, and optional relay allocation behind a small host-facing lifecycle.

Relay-backed reachability is required by default, and startup returns only
after at least one relay reservation has produced a confirmed publication
route. `AukiPeerConfig::direct_only()` is the explicit opt-out and makes no DMS
booking calls.

The public runtime accepts an `auki-auth` `PreparedPeer` and owns its single
renewal loop. It pins the Domain and Peer, installs verification keys before
credentials, publishes only a sensitive revisioned relay header, coalesces DMS
401 refresh requests, and fences authorization at literal credential expiry.
External push-authority integration remains a later public API slice.
Authentication and product-specific heartbeat policy remain outside this
crate.

Confirmed relay routes are fenced in a local route catalog and are removed on
assignment changes, transport loss, authority expiry, or bounded shutdown.
Explicit `AukiPeer::shutdown()` drains reservations and deletes the DMS booking
while authority and Domain are still alive, then stops authority and leaves the
Domain. Dropping a runtime or canceling startup only fences local resources; it
does not own a DMS `DELETE`, so a booking created at that boundary expires via
its requester-authority TTL.

One live runtime per Peer ID is the supported deployment invariant. Sequential
restart with the same stable identity is supported; simultaneous processes or
pods using the same identity are not. The SDK does not add distributed leasing
or controller fencing for that unsupported topology.

On Kubernetes, use one replica per persisted identity (for example, a
single-replica StatefulSet or a Recreate rollout); parallel replicas must use
distinct identities.
