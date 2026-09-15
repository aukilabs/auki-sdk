# Sign in and choose a Domain

You need credentials and a Domain ID that the account can access.
[Add the SDK to your app](../reference/networking.md#platforms-and-installation)
before using these native Rust examples.

## Sign in with a User account

Set `AUKI_EMAIL`, `AUKI_PASSWORD`, and `AUKI_DOMAIN_ID` as in the
[tutorial](../tutorials/first-peer.md). This complete program signs in to the
development services, prints its Peer ID, and shuts down:

~~~rust
use auki_sdk::{AukiPeerBootstrap, Credentials, DomainSelection};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let bootstrap = AukiPeerBootstrap::dev(Credentials::user_password(
        std::env::var("AUKI_EMAIL")?,
        std::env::var("AUKI_PASSWORD")?,
    ))
    .await?;

    let domain = std::env::var("AUKI_DOMAIN_ID")?.parse()?;
    let peer = bootstrap
        .start_persistent_peer(DomainSelection::new(domain), "./state/peer.identity")
        .await?;

    println!("peer: {}", peer.peer_id());
    let cleanup = peer.shutdown().await;
    bootstrap.session().close().await;
    cleanup?;
    Ok(())
}
~~~

To show a Domain picker, call `bootstrap.accessible_domains().await?` before
starting the peer. Pass the selected `choice.domain.id` to `DomainSelection::new`.

To reuse this login for HTTP data access, pass a cloned session to the
[Domain data clients](domain-data.md). Close all clients and peers before
closing the session.

## Sign in as a backend service

Use `Credentials::app(access_key, secret)` in place of User credentials above.
Keep App secrets on the backend; do not embed them in browser or mobile apps.

## Connect to another environment

Get the API, DDS, and DMS URLs from your environment administrator. Replace
`AukiPeerBootstrap::dev` with this function:

~~~rust
use auki_sdk::{AuthClient, AuthEnvironment, AukiPeerBootstrap, AukiPeerConfig, Credentials};

async fn authenticate(
    api_base: &str,
    dds_base: &str,
    dms_base: &str,
    credentials: Credentials,
) -> anyhow::Result<AukiPeerBootstrap> {
    Ok(AukiPeerBootstrap::authenticate(
        AuthClient::new(AuthEnvironment::new(api_base, dds_base)?)?,
        credentials,
        AukiPeerConfig::new(dms_base)?,
    )
    .await?)
}
~~~

Replace `AukiPeerConfig::new(dms_base)?` with your
[connection configuration](connect.md#accept-connections-directly-or-through-a-relay)
if you need direct listeners or different relay settings.

## Reuse a ZITADEL login

Import your app's PKCE login with its access token, refresh token, trusted
issuer, client ID, and optional expiry. The SDK will refresh the tokens;
stop any other refresh loop for that login.

In Rust, call `AuthClient::import_zitadel_session` with a `ZitadelSessionStore`,
then `AukiPeerBootstrap::from_session` if you need a peer. Web uses
`AukiUserSession.importZitadelDev`; Python uses `AukiSession.import_zitadel_dev`.
Swift and Expo also support import. The same session supports
[Domain selection and data access](domain-data.md#reuse-an-imported-login). See the
[Expo example](../../core/bindings/expo/README.md#import-an-existing-login).

Your storage callback must save all replacement credentials together and await
every write, including on failure. Keep the session if startup or saving fails.
On a `persistence` error, retry with that session to save its retained tokens.

Imported listing requires an API deployment that implements the
`purpose=p2p` human Domain-allowlist exchange and DDS that accepts its
`user-p2p-access` token on `/accessible-domains`. The SDK rejects older exchanges
that ignore the purpose and preserves HTTP 403 when the account has no readable
Domains. It does not fall back to organization-wide listing.

A known Domain ID can still use the separate data path: the API deployment must
accept ZITADEL for the ordinary service exchange. DDS must separately support
direct ZITADEL login for P2P. On logout, close data clients and peers, close the
session, then delete stored credentials. See the
[deployment evidence and limits](../../test-support/domain-data-validation.md#provider-compatibility).

## Build a compute node or robot

Use provisioned DDS credentials for a compute node or robot. Compute nodes
also need a signing wallet; robots use the deployment's robot audience.
These credentials are separate from User and App login.

See [Run compute and robot tasks](run-compute-tasks.md) for Rust and Python
setup, and [apps, services, compute nodes, and robots](../explanation/apps-nodes-and-robots.md)
for Domain and assignment rules.
