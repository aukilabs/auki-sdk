# Authenticate and select a Domain

Use an authentication session to authorize one peer identity in one selected
Domain. Start with the [installation reference](../reference/networking.md#platforms-and-installation)
if the SDK is not yet a dependency of your app.

## User or service credentials

This complete native Rust example reads a User login and a known Domain from
the environment, starts a peer, and stops it:

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

For a trusted backend service, replace `Credentials::user_password(...)` with
`Credentials::app(access_key, secret)`. App secrets belong in trusted service
storage; do not embed them in browser or mobile applications.

To offer a Domain picker, call `bootstrap.accessible_domains().await?` before
starting. Pass the selected `choice.domain.id` to `DomainSelection::new`.

The `dev` constructor selects development services. To use your own environment,
replace its construction with:

~~~rust
use auki_sdk::{AuthClient, AuthEnvironment, AukiPeerBootstrap, AukiPeerConfig};

async fn authenticate(
    api_base: &str,
    dds_base: &str,
    dms_base: &str,
    credentials: auki_sdk::Credentials,
) -> anyhow::Result<AukiPeerBootstrap> {
    Ok(AukiPeerBootstrap::authenticate(
        AuthClient::new(AuthEnvironment::new(api_base, dds_base)?)?,
        credentials,
        AukiPeerConfig::new(dms_base)?,
    )
    .await?)
}
~~~

Use service URLs supplied by your environment administrator.

## An existing ZITADEL login

The host performs PKCE login and supplies its access token, refresh token,
trusted issuer, client ID, and optional token expiry. Hand refresh ownership to
the SDK and retain the imported session if startup fails.

In Rust, use `AuthClient::import_zitadel_session` with a
`ZitadelSessionStore`, then `AukiPeerBootstrap::from_session`. Web exposes
`AukiUserSession.importZitadelDev`; Swift and Expo expose equivalent imports.
See the [Expo import example](../../core/bindings/expo/README.md#import-an-existing-login).

The storage callback must atomically save the complete replacement credentials
and finish all its writes before returning success or failure. Await persistence;
keep only one refresh owner for a grant. On a `persistence` error, retry using
the same session so its retained replacement can be saved.

Supply a known Domain ID: imported ZITADEL sessions currently do not support
`accessible_domains`. The DDS environment must support direct ZITADEL P2P
admission. On logout, stop peers, await session close, then clear stored
credentials.

## A host that already manages machine authentication

Native hosts may call `AukiPeer::start_external(identity, update, config)`.
Retain its returned authority-control handle, supply complete replacements with
`replace`, and service `next_refresh_request`. The SDK still owns the
networking lifecycle.

Posemesh already composes this for its
[robot and compute runners](https://github.com/aukilabs/posemesh/tree/main/core/compute-node).
Use that runner layer when building a task worker.
