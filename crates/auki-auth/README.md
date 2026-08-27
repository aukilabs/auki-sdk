# auki-auth

`auki-auth` turns Auki API credentials into the authority needed to start one
authenticated P2P peer. It handles API/DDS HTTP, accessible-Domain selection,
proof of the peer's stable libp2p identity, verification keys, and the signed
Domain credential.

It deliberately does not discover peers, resolve routes, contact DMS, book a
relay, join a Domain, or spawn a renewal task. The host remains the composition
root for those operations.

## Authenticate in dev

For a person using a trusted native application:

```rust,no_run
let client = auki_auth::AuthClient::new(auki_auth::AuthEnvironment::dev())?;
let session = client
    .authenticate(auki_auth::Credentials::user_password(
        "developer@example.com",
        password_from_secret_store,
    ))
    .await?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

For a trusted native or headless application:

```rust,no_run
let client = auki_auth::AuthClient::new(auki_auth::AuthEnvironment::dev())?;
let session = client
    .authenticate(auki_auth::Credentials::app(
        app_access_key,
        app_secret_from_secret_store,
    ))
    .await?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

An app secret must not be embedded in a browser or mobile frontend. Both
authentication methods return the same `AuthSession`; everything after this
point is identical.

## Prepare and start a Domain

Persist one identity and reuse it on every launch. Missing, corrupt, or unsafe
identity material fails closed instead of silently changing the Peer ID.

```rust,no_run
use auki_auth::DomainSelection;
use auki_domain::{Domain, DomainConfig};
use auki_p2p::Identity;
use auki_session::Peer;

let selected = session
    .accessible_domains()
    .await?
    .into_iter()
    .next()
    .ok_or("no accessible Domain")?;
let identity = Identity::load_or_create("./state/auki-peer.identity")?;
let prepared = session
    .authorize_peer(
        DomainSelection::new(selected.domain.id),
        &identity.proof(),
    )
    .await?;

let peer = Peer::new(prepared.peer_id.to_string(), "robot-experiment")
    .with_storage_root("./state".into());
let sdk_session = peer.start_session()?;
let domain = Domain::builder(
    &peer,
    &sdk_session,
    DomainConfig::new(prepared.domain.id, identity),
)
.authority(
    prepared.verification_keys.clone(),
    prepared.initial_credential.clone(),
)
.join()
.await?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

This starts with no discovered routes. Configure listeners and exact-peer route
candidates on `DomainConfig` when the host has them.

## Renew explicitly

The host decides when to renew, normally using `prepared.renew_at`. Install the
new key set before the new credential on the existing Domain:

```rust,no_run
let renewed = prepared.renewal.renew().await?;
let authority = domain.authority();
authority
    .install_verification_keys(renewed.verification_keys)
    .await?;
authority.install_credential(renewed.credential).await?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

This does not reconnect or reconstruct the Domain. `auki-auth` does not sleep,
schedule, perform implicit transient retries, or spawn a global runtime;
cancellation-aware variants are available for hosts that own shutdown. A valid
rotation retains the old signer as `previous`, so cancellation after the key
update leaves the old credential usable. The host runtime should retry a failed
renewal within a bounded budget before literal credential expiry, then stop or
explicitly fence the peer if authority cannot be renewed.
