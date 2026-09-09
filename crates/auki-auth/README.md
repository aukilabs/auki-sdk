# auki-auth

`auki-auth` turns an Auki User or trusted App login into a validated
`PreparedPeer`: authority for one exact Peer ID in one selected DDS Domain.

```text
credentials -> AuthSession -> selected Domain + identity proof -> PreparedPeer
                                                                  |
                                                                  v
                                                           AukiPeer::start
```

The User/App preparation API owns bounded API/DDS exchanges, accessible-Domain listing, Peer-ID
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

`authorize_peer` proves access to the selected Domain through DDS challenge
and verify. 403 and 404 on those endpoints map to `DomainNotAccessible`. Call
`accessible_domains()` first when the application needs to present a list to a
person.

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

## ZITADEL sessions

`ZitadelSessionCredentials::new(access_token, refresh_token, client_id, issuer,
access_token_expires_at)` accepts a public-client session after the host's PKCE
login. Tokens can be opaque; initial expiry is an optional UTC timestamp. Keep
issuer/client ID in trusted application configuration, not unverified token claims.
Credential diagnostics are redacted. Reading tokens requires the explicit
`access_token().expose_secret()` / `refresh_token().expose_secret()` accessors;
never log those values.

Configure initial login with `offline_access`, DDS's configured ZITADEL audience,
and the `auki-api-organization` metadata mapping. The SDK does not add scopes or
change organizations during refresh. See the official ZITADEL
[endpoint contract](https://zitadel.com/docs/apis/openidoauth/endpoints) and
[scope configuration](https://zitadel.com/docs/apis/openidoauth/scopes).

`ZitadelTokenClient` is the low-level refresh primitive, not a second session
owner. It validates discovery against the expected issuer, permits only its
same-origin token endpoint, and sends public-client authentication `none` with
no scope or client secret. HTTPS is required except for explicit loopback HTTP
development URLs. Redirects are rejected before following them on native and
Wasm. Browser deployments need provider CORS support. Requests and streamed
response bodies are bounded by `AuthLimits`.

The result retains a replacement refresh token when supplied (otherwise the
previous one), with UTC access expiry computed from `expires_in`. A request whose
rotation outcome cannot be recovered returns `RefreshOutcomeUnknown`: require
login instead of replaying the old token. Provider error descriptions and
transport internals are not exposed in errors.

`ZitadelSessionStore::save` is the session-scoped persistence boundary: atomically
save all five fields to secure host storage and acknowledge only after completion.
Reject with `Error::Persistence` without including raw host error text. Do not
implement storage acknowledgement as a fire-and-forget event. Host OAuth
libraries, other tabs, and other processes must stop refreshing a handed-over
session; an SDK coordinator cannot serialize independent refresh owners.

Ordinary applications import a session synchronously, then start a peer through
the existing bootstrap. Import performs no network work, so the host retains a
recoverable handle even if startup rotates credentials and subsequently fails:

```rust,no_run
use std::sync::Arc;
use auki_auth::{AuthClient, DomainSelection, ZitadelSessionCredentials, ZitadelSessionStore};
use auki_sdk::{AukiPeerBootstrap, AukiPeerConfig, Identity};

async fn run(
    client: AuthClient,
    credentials: ZitadelSessionCredentials,
    store: Arc<dyn ZitadelSessionStore>,
    domain: DomainSelection,
    identity: Identity,
) -> Result<(), Box<dyn std::error::Error>> {
    let session = client.import_zitadel_session(credentials, store)?;
    let bootstrap = AukiPeerBootstrap::from_session(session.clone(), AukiPeerConfig::dev());
    // The application supplies the Domain ID; ZITADEL v1 does not list Domains.
    let result = bootstrap.start_peer(domain, identity).await;
    // Keep `session` available to retry after transient or persistence failure.
    if let Ok(peer) = result { peer.shutdown().await?; }
    session.close().await;
    // Only now may the host clear its secure session storage for logout.
    Ok(())
}
```

Before DDS admission, known expiry within 30 seconds triggers refresh; unknown
expiry first tries DDS and recovers from one 401. Both share one refresh budget
per operation. ZITADEL uses the access token directly on DDS's
`/api/v1/domains/{domainID}/p2p/zitadel/challenge` and `/verify` routes. DDS verifies
the identity, checks effective Domain `domain_metadata_read` through policy
`/check`, verifies live DDS ownership, and binds its final token to the peer key.
No API exchange, API Domain catalog, or policy legacy listing participates.
`accessible_domains()` returns a configuration error for ZITADEL sessions;
password/app discovery and exchanges are unchanged. A DDS 401 rotates the access
token and restarts the complete bearer-bound proof once, using the same Peer ID.
Domain denial does not poison other Domains sharing the session.

One session-owned refresh/save task survives caller cancellation and supervisor
timeouts. Replacements enter memory before storage is awaited. A rejected save
returns `Error::Persistence`; the next operation saves that same generation
before any rotation or DDS admission request.
`Error::kind()` exposes login-required, configuration, Domain denial, persistence,
transient, cancelled and closed recovery categories. Invalid grants, ambiguous
refresh submissions, or a rejected freshly refreshed access token latch a
login-required state; correcting configuration requires importing a new session.

Each HTTP request and each caller's wait is bounded by `AuthLimits`.
`SessionOperationPending` means retry waiting on the existing task, not launch
another refresh. An already-running host write cannot safely be cancelled by the
SDK: `close()` fences all clones and drains its acknowledgement before returning.
If the close waiter is cancelled, await `close()` again before clearing storage.
The storage callback must settle all writes before success **or failure**, must
not reenter the session, and must eventually acknowledge; a never-resolving save
prevents safe logout completion. Closing a session prevents future renewal; hosts
also shut down their peers. It does not revoke existing remotely held tokens.

A caller using the low-level token client must drive an issued refresh to
completion and retain/save the result before another refresh. A process crash
between rotation and durable storage can require login. Provider refresh-token
idle/absolute limits must cover expected inactive periods; the SDK cannot extend
them. There is no additional auth scheduler or shorter revocation lifetime.

An active peer is not a provider-refresh heartbeat. Each P2P renewal rechecks DDS
admission using the current ZITADEL access token, refreshing it only when needed.
Configure the provider's refresh-token idle lifetime for the access-token lifetime
plus expected inactive periods. Already-issued 30-minute P2P tokens retain their
existing expiry/revocation delays; this release adds no immediate revocation hook.

## Web/Wasm

User authentication and authority preparation compile to Wasm. The generic Web
binding exposes them as `AukiUserSession`: JavaScript logs in a User, optionally
lists accessible Domains, and starts an `AukiPeer` in a selected Domain.

The Web `0.1` facade creates a fresh in-memory identity for each peer start.
Relay-backed mode acquires one confirmed WSS relay reservation; outbound-only
mode skips the booking and exposes no local route. Reloading or starting again
creates a new Peer ID. The facade does not accept App credentials or persist
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

## Native machine operations

`auki_auth::machine` contains the existing native Node/Robot building blocks.
They take explicit inputs and do not depend on Posemesh configuration or a runner.

| Module | Responsibility |
| --- | --- |
| `robot` | Registration-secret register/verify through `RobotAuthenticator` |
| `token_manager` | Shared access cache, refresh/retry and `TokenProvider` |
| `registration` | One-shot signed Node registration and crypto helpers |
| `siwe` | Node nonce, message, wallet signature and verification |
| `p2p` | Machine bearer/Peer ID proof, verification keys and Robot P2P exchange |

The compile-checked [Robot example](examples/robot_token.rs) authenticates with
`DDS_BASE_URL`, `ROBOT_REGISTRATION_CREDENTIALS` and `AUKI_CAPABILITY`.
`bearer()` works on demand; `start_bg()` explicitly enables refresh and
`stop_bg()` stops it. Authentication itself does not start a peer or poll DMS.

The [Node example](examples/node_token.rs) takes `DDS_BASE_URL`, `REG_SECRET`,
`SECP256K1_PRIVHEX` and `AUKI_CAPABILITY`. Node login still uses a wallet and
SIWE after registration. A host owns readiness callbacks, registration-loop
timing and re-arming after 403/404; SDK operations do not mutate global state.
The example makes one registration attempt and one subsequent login attempt.

`machine::p2p::PeerBindingClient` proves the same identity using the exact base
bearer for both challenge and verify. Hosts compose it with their authenticator
when peer-bound machine access is required, including after reauthentication.
`DdsP2pClient::robot_p2p_token` uses the current assigned Domain as a request hint;
DDS validates the signed token and assignment. `authority_material` returns keys
and a credential for a host runtime to validate and install. It starts no renewal
driver and does not grant access by itself.

Run the auth examples explicitly with `cargo run -p auki-auth --example robot_token`
or `--example node_token` after supplying the inputs. They contact DDS and never
print credentials or returned tokens.

Use `auki-dms` directly for task HTTP/polling in a custom native application.
Use Posemesh's compute-node when its existing Runner, task supervision, heartbeat
scheduling, storage ports and graceful shutdown fit the host. Its auth/token-manager
and SIWE paths re-export SDK blocks; Robot/Node startup wrappers, registration
readiness and the Robot peer-renewal driver remain in Posemesh.

These machine APIs are native Rust only. Existing User/App/ZITADEL APIs and
bindings retain their current behavior.
