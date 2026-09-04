# auki-sdk

`auki-sdk` is the mechanical runtime facade for authenticated Auki peers. An
`AukiPeer` composes one stable identity and either pulled or externally managed
authority with an authenticated `auki-p2p` node, configured routes, credential
renewal, application protocols, observations, and optional relay allocation
behind a small host-facing lifecycle.

Relay-backed reachability is configured by default, and that mode returns from
startup only after at least one relay reservation has produced a confirmed
TCP/WSS route pair. One booking requests `relay_count` provider slots; each
slot owns one provider/reservation and one route pair. The count is provider
redundancy, not transport count. `AukiPeerConfig::without_relay()` is the
cross-target opt-out and makes no DMS booking calls;
`AukiPeerConfig::direct_only()` is its native alias. Zero listeners and
advertised routes are valid for an outbound-only native peer; accepting inbound
direct connections requires a listener plus a dialable route shared by the
application. Configure an advertised direct route only when the application
uses the SDK's local route catalog as that sharing source. Relay-backed browser
peers establish their reservation over WSS; native and Python peers establish
it over TCP. An outbound-only browser exposes no local route but can dial a
remote peer's WSS relay route. Web bootstrap uses a session-ephemeral identity.

Native and browser targets expose the same application-facing
`AukiPeer`/`AukiPeerConfig` and
`AukiPeerProtocols`/`AukiProtocolSpec` names. The browser implementation runs
local, non-`Send` protocol handlers and keeps each exact relay circuit used for
an outbound stream alive for that authenticated stream's lifetime.
Platform-specific transport and executor details stay behind that facade.

Both targets also expose a clone-only `AukiPeerLifecycle`. Its
`wait_stopped()` operation returns the same target-neutral `AukiPeerExit`, and
the observer remains usable after `shutdown(self)` consumes the peer. An
unexpected terminal failure fences retained protocol handles before publishing
the failure. The browser lifecycle supervisor renews authority with or without
a local relay booking. Native status subscriptions remain an additional
diagnostic surface rather than a binding requirement.

## User/App bootstrap

`AukiPeerBootstrap` is the ordinary application entry point. It keeps the
User/App authentication session and peer configuration together, while the
application still selects one explicit Domain and identity policy:

```rust,no_run
use auki_sdk::{AukiPeerBootstrap, Credentials, DomainSelection};

# async fn start() -> Result<(), Box<dyn std::error::Error>> {
let bootstrap = AukiPeerBootstrap::dev(Credentials::user_password(
    "developer@example.com",
    "password-from-a-secret-store",
))
.await?;

// Present bootstrap.accessible_domains().await? when a person must choose.
let domain_id = std::env::var("AUKI_DOMAIN_ID")?.parse()?;
let peer = bootstrap
    .start_persistent_peer(
        DomainSelection::new(domain_id),
        "./state/auki-peer.identity",
    )
    .await?;

// Mount exact protocol endpoints through peer.protocols().
peer.shutdown().await?;
# Ok(())
# }
```

`start_peer` accepts an application-supplied `Identity`, which is the canonical
path for platform-managed stores such as iOS Keychain. Web uses
`start_ephemeral_peer` intentionally. Native applications normally use the
fail-closed persistent helper above. Trusted native App credentials use the
same facade by replacing `Credentials::user_password` with `Credentials::app`.

`PreparedPeer` and raw `AukiPeer::start` remain available for lower-level
composition and tests.

`AukiPeer::start` accepts an `auki-auth` `PreparedPeer` and owns its single pull
renewal loop. On native targets, `AukiPeer::start_external` instead accepts one
complete `ExternalAuthorityUpdate` and returns the sole
`ExternalAuthorityControl` for subsequent replacements and coalesced DMS 401
refresh requests. Both paths pin the Domain and Peer ID, install verification
keys before credentials, publish only a sensitive revisioned relay header, and
fence authorization at literal credential expiry. `AukiPeerBootstrap` owns
User/App authentication and renewal preparation; product-specific heartbeat
policy remains outside this crate.

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
local handlers before releasing any relay it owns. On native targets,
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
