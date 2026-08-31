# auki-sdk

`auki-sdk` is the mechanical runtime facade for authenticated Auki peers. An
`AukiPeer` composes one stable identity and either pulled or externally managed
authority with an authenticated `auki-p2p` node, configured routes, credential
renewal, application protocols, observations, and optional relay allocation
behind a small host-facing lifecycle.

Relay-backed reachability is required by default, and startup returns only
after at least one relay reservation has produced a confirmed publication
route. `AukiPeerConfig::direct_only()` is the explicit opt-out and makes no DMS
booking calls.

`AukiPeer::start` accepts an `auki-auth` `PreparedPeer` and owns its single pull
renewal loop. `AukiPeer::start_external` instead accepts one complete
`ExternalAuthorityUpdate` and returns the sole `ExternalAuthorityControl` for
subsequent replacements and coalesced DMS 401 refresh requests. Both paths pin
the Domain and Peer ID, install verification keys before credentials, publish
only a sensitive revisioned relay header, and fence authorization at literal
credential expiry. Authentication, refresh scheduling, and product-specific
heartbeat policy remain outside this crate.

External control is returned only after startup readiness. Consequently an
external-authority runtime that enables relay booking cannot answer a DMS 401
during startup: that attempt fails within the authority refresh deadline, and
the host must obtain fresh authority and retry using its retained or reloaded
stable identity. Direct-only external Compute peers avoid this relay-startup
constraint.

Confirmed relay routes are fenced in a local route catalog and are removed on
assignment changes, transport loss, authority expiry, or bounded shutdown.
Explicit `AukiPeer::shutdown()` drains reservations and deletes the DMS booking
while authority and transport are still alive, then stops managed protocols,
authority, and the transport. Dropping a runtime or canceling startup only
fences local resources; it does not own a DMS `DELETE`, so a booking created at
that boundary expires via its requester-authority TTL.

One live runtime per Peer ID is the supported deployment invariant. Sequential
restart with the same stable identity is supported; simultaneous processes or
pods using the same identity are not. The SDK does not add distributed leasing
or controller fencing for that unsupported topology.

On Kubernetes, use one replica per persisted identity (for example, a
single-replica StatefulSet or a Recreate rollout); parallel replicas must use
distinct identities.
