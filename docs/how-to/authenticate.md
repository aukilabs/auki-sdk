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
then `AukiPeerBootstrap::from_session`. Web uses `AukiUserSession.importZitadelDev`;
Swift and Expo also support import. See the
[Expo example](../../core/bindings/expo/README.md#import-an-existing-login).

Your storage callback must save all replacement credentials together and await
every write, including on failure. Keep the session if startup or saving fails.
On a `persistence` error, retry with that session to save its retained tokens.

Provide a known Domain ID: imported sessions cannot currently list Domains.
Your DDS deployment must support ZITADEL login for P2P access. On logout, stop
peers, close the session, then delete stored credentials.

## Build a robot or compute worker

[Posemesh runners](https://github.com/aukilabs/posemesh/tree/main/core/compute-node)
already manage machine authentication and execution for DMS tasks. Build your
worker on that runner.
