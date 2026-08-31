# auki-sdk

`auki-sdk` is the mechanical runtime facade for authenticated Auki peers. An
`AukiPeer` composes one stable identity and either pulled or externally managed
authority with an authenticated `auki-p2p` node, configured routes, credential
renewal, application protocols, observations, and optional relay allocation
behind a small host-facing lifecycle.

Relay-backed reachability is required by default, and startup returns only
after at least one relay reservation has produced a confirmed publication
route. On native targets, `AukiPeerConfig::direct_only()` is the explicit
opt-out and makes no DMS booking calls. Zero listeners and advertised routes
are valid for an outbound-only direct peer; accepting inbound direct
connections requires a listener and a matching externally reachable route.
Browser peers always own one WSS relay reservation and use a
session-ephemeral identity supplied by the application.

Native and browser targets expose the same application-facing
`AukiPeer`/`AukiPeerConfig` and
`AukiPeerProtocols`/`AukiProtocolSpec` names. The browser implementation runs
local, non-`Send` protocol handlers and keeps each exact relay circuit alive
for the lifetime of its authenticated stream. Platform-specific transport and
executor details stay behind that facade.

`AukiPeer::start` accepts an `auki-auth` `PreparedPeer` and owns its single pull
renewal loop. On native targets, `AukiPeer::start_external` instead accepts one
complete `ExternalAuthorityUpdate` and returns the sole
`ExternalAuthorityControl` for subsequent replacements and coalesced DMS 401
refresh requests. Both paths pin the Domain and Peer ID, install verification
keys before credentials, publish only a sensitive revisioned relay header, and
fence authorization at literal credential expiry. Authentication, refresh
scheduling, and product-specific heartbeat policy remain outside this crate.

External control is returned only after startup readiness. Consequently an
external-authority runtime that enables relay booking cannot answer a DMS 401
during startup: that attempt fails within the authority refresh deadline, and
the host must obtain fresh authority and retry using its retained or reloaded
stable identity. Direct-only external Compute peers avoid this relay-startup
constraint.

Native confirmed routes are fenced in a local route catalog and are removed on
assignment changes, transport loss, authority expiry, or bounded shutdown.
Explicit `AukiPeer::shutdown()` is the cleanup barrier for managed protocols,
relay reservations, authority, and transport. The native runtime drains relay
reservations before stopping managed protocols; the browser runtime stops
local handlers before releasing its mandatory relay. On native targets,
dropping a runtime or canceling startup only fences local resources; a booking
created at that boundary expires via its requester-authority TTL. Browser drop
signals its local supervisor to attempt the same booking cleanup asynchronously.

One live runtime per Peer ID is the supported deployment invariant. Sequential
restart with the same stable identity is supported; simultaneous processes or
pods using the same identity are not. The SDK does not add distributed leasing
or controller fencing for that unsupported topology.

On Kubernetes, use one replica per persisted identity (for example, a
single-replica StatefulSet or a Recreate rollout); parallel replicas must use
distinct identities.
