# auki-auth

`auki-auth` turns an Auki User or trusted App login into a validated
`PreparedPeer`: authority for one exact Peer ID in one selected DDS Domain.

```text
credentials -> AuthSession -> selected Domain + identity proof -> PreparedPeer
                                                                  |
                                                                  v
                                                           AukiPeer::start
```

The crate owns bounded API/DDS exchanges, accessible-Domain validation, Peer-ID
proof, verification keys, and the initial signed credential. It deliberately
does not discover peers, resolve or publish routes, contact DMS, book a relay,
or spawn an authority-renewal task.

The high-level `auki_sdk::AukiPeer` consumes `PreparedPeer` and owns renewable
authority, transport, relay-backed reachability, protocols, fencing, and
shutdown.

## Native Rust

Native applications may authenticate a User or a trusted App. They normally
persist one identity and reuse it on every launch:

```rust,no_run
use std::env;

use auki_auth::{AuthClient, AuthEnvironment, Credentials, DomainSelection};
use auki_sdk::{AukiPeer, AukiPeerConfig, Identity};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let email = env::var("AUKI_EMAIL")?;
    let password = env::var("AUKI_PASSWORD")?;
    let domain_id = env::var("AUKI_DOMAIN_ID")?.parse()?;
    let identity = Identity::load_or_create("./state/auki-peer.identity")?;
    let session = AuthClient::new(AuthEnvironment::dev())?
        .authenticate(Credentials::user_password(email, password))
        .await?;
    let prepared = session
        .authorize_peer(DomainSelection::new(domain_id), &identity.proof())
        .await?;

    let peer = AukiPeer::start(identity, prepared, AukiPeerConfig::dev()).await?;
    // Mount product protocols through peer.protocols().
    peer.shutdown().await?;
    Ok(())
}
```

`authorize_peer` verifies that the authenticated principal can currently access
the selected Domain. Call `accessible_domains()` first when the application
needs to present a list to a person.

Identity material fails closed if it is missing in an unsafe state or corrupt;
the SDK never silently replaces corrupt material with a new Peer ID. One
persisted identity belongs to one live runtime at a time.

For a trusted native or headless application, only the authentication input
changes:

```rust,no_run
use std::env;

use auki_auth::{AuthClient, AuthEnvironment, Credentials};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let app_access_key = env::var("AUKI_APP_ACCESS_KEY")?;
    let app_secret = env::var("AUKI_APP_SECRET")?;
    let client = AuthClient::new(AuthEnvironment::dev())?;
    let _session = client
        .authenticate(Credentials::app(app_access_key, app_secret))
        .await?;
    Ok(())
}
```

Never embed an App secret in a browser, mobile binary, public repository,
container image, or log.

## Web/Wasm

User authentication and authority preparation compile to Wasm. The generic Web
binding exposes them as `AukiUserSession`: JavaScript logs in a User, lists
accessible Domains, and selects one before starting an `AukiPeer`.

The Web `0.1` facade creates a fresh in-memory identity for each peer start and
always acquires one confirmed WSS relay route. Reloading or starting again
therefore creates a new Peer ID. It does not accept App credentials or persist
the User password or peer identity.

See the
[browser echo app](../../examples/portable-echo/web/README.md#run-the-web-app)
for the complete public flow.

## Boundary rules

- `auki-auth` proves authority; it does not create reachability.
- `AukiPeer` renews authority and owns relay-backed runtime lifecycle.
- Native peers normally persist identity; Web peers are intentionally ephemeral
  in `0.1`.
- A remote Peer ID and exact TCP or WSS route still come from application
  configuration, a product control plane, or manual exchange.
- A route is never authority, and `0.1` has no automatic discovery or route
  publication.

Low-level hosts may consume `PreparedPeer::renewal` themselves, but ordinary
User/App applications should use `AukiPeer::start` rather than reimplementing
key rotation, credential expiry fencing, relay recovery, and cleanup.
