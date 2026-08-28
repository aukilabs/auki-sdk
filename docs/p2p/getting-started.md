# Getting started with Auki P2P in Rust

This guide starts a real authenticated peer on the Auki dev environment using
the high-level `AukiPeer` facade. By the end, it will have:

- a persistent Peer ID;
- authority for one selected DDS Domain;
- automatic credential renewal;
- a confirmed relay route;
- a tiny authenticated echo protocol; and
- clean, ordered shutdown.

You can then start a second peer, give it the first peer's ID and route, and
exchange `ping`/`pong` over an authenticated stream.

## Before you begin

You need:

- Rust `1.89.0` or newer;
- an Auki User account with access to a DDS Domain;
- that Domain's UUID; and
- two terminals for the two-peer test.

App access-key/secret authentication uses the same peer lifecycle and is shown
later. Never ship an App secret in a browser or mobile application.

## 1. Create a Rust application

```sh
cargo new auki-peer-demo
cd auki-peer-demo
```

Until `v0.1.0` is tagged, pin both Auki crates to the same reviewed revision:

```toml
# Cargo.toml
[package]
name = "auki-peer-demo"
version = "0.1.0"
edition = "2024"
rust-version = "1.89"

[dependencies]
anyhow = "1"
auki-auth = { git = "https://github.com/aukilabs/auki-sdk", rev = "027a1c76224079036fef7f4e3d4c8353a0001bd0" }
auki-sdk = { git = "https://github.com/aukilabs/auki-sdk", rev = "027a1c76224079036fef7f4e3d4c8353a0001bd0" }
futures = "0.3"
tokio = { version = "1", features = ["macros", "rt-multi-thread", "signal", "time"] }
uuid = "1"
```

Do not mix Auki crate revisions. Identity, authority, Domain, and protocol
contracts form one coordinated release line.

## 2. Start one peer

Replace `src/main.rs` with:

```rust
use std::{env, path::PathBuf, time::Duration};

use anyhow::{Context, Result, bail};
use auki_auth::{AuthClient, AuthEnvironment, Credentials, DomainSelection};
use auki_sdk::{AukiPeer, AukiPeerConfig, DomainProtocolSpec, Identity, Multiaddr, PeerId};
use futures::{AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;

const ECHO_PROTOCOL: &str = "/auki/example/echo/1.0.0";

#[tokio::main]
async fn main() -> Result<()> {
    let email = env::var("AUKI_EMAIL").context("AUKI_EMAIL is required")?;
    let password = env::var("AUKI_PASSWORD").context("AUKI_PASSWORD is required")?;
    let domain_id: Uuid = env::var("AUKI_DOMAIN_ID")
        .context("AUKI_DOMAIN_ID is required")?
        .parse()
        .context("AUKI_DOMAIN_ID must be a UUID")?;
    let state_dir =
        PathBuf::from(env::var("AUKI_STATE_DIR").unwrap_or_else(|_| "./state".to_owned()));

    let auth = AuthClient::new(AuthEnvironment::dev())?;
    let auth_session = auth
        .authenticate(Credentials::user_password(email, password))
        .await?;
    let selected = auth_session
        .accessible_domains()
        .await?
        .into_iter()
        .find(|choice| choice.domain.id == domain_id)
        .context("the authenticated User cannot access AUKI_DOMAIN_ID")?;

    // Reuse this file on every launch. Losing it creates a different Peer ID.
    let identity = Identity::load_or_create(state_dir.join("peer.identity"))?;
    let prepared = auth_session
        .authorize_peer(DomainSelection::new(selected.domain.id), &identity.proof())
        .await?;

    // Relay-backed reachability is enabled by default in the dev config.
    let mut config = AukiPeerConfig::dev("auki-peer-demo", state_dir);
    let remote_peer = match (
        env::var("AUKI_REMOTE_PEER_ID"),
        env::var("AUKI_REMOTE_ROUTE"),
    ) {
        (Ok(peer_id), Ok(route)) => {
            let peer_id: PeerId = peer_id.parse().context("invalid remote Peer ID")?;
            let route: Multiaddr = route.parse().context("invalid remote route")?;
            config = config.with_peer_routes(peer_id, [route])?;
            Some(peer_id)
        }
        (Err(_), Err(_)) => None,
        _ => bail!("set both AUKI_REMOTE_PEER_ID and AUKI_REMOTE_ROUTE, or neither"),
    };

    // This returns after authority, Domain, and one relay route are ready.
    let peer = AukiPeer::start(identity, prepared, config).await?;
    let context = peer.protocol_context();

    // Custom protocols are explicit. Keep the registration alive while serving.
    let _echo = context.protocols().register(
        DomainProtocolSpec::new(ECHO_PROTOCOL, 32, 4)?,
        |mut stream| async move {
            let exchange = async {
                let mut request = [0_u8; 4];
                stream.read_exact(&mut request).await?;
                if &request == b"ping" {
                    stream.write_all(b"pong").await?;
                    stream.flush().await?;
                }
                Ok::<(), std::io::Error>(())
            };
            let _ = tokio::time::timeout(Duration::from_secs(5), exchange).await;
        },
    )?;

    println!("READY");
    println!("  domain: {}", peer.domain_id());
    println!("  peer:   {}", peer.peer_id());
    for published in context.routes().snapshot()?.relay_routes {
        println!("  route:  {}", published.route);
    }

    if let Some(remote_peer) = remote_peer {
        let exchange = async {
            let mut stream = context.protocols().open(remote_peer, ECHO_PROTOCOL).await?;
            stream.write_all(b"ping").await?;
            stream.flush().await?;
            let mut response = [0_u8; 4];
            stream.read_exact(&mut response).await?;
            anyhow::ensure!(&response == b"pong", "unexpected echo response");
            stream.close().await?;
            Ok::<(), anyhow::Error>(())
        };
        tokio::time::timeout(Duration::from_secs(5), exchange)
            .await
            .context("echo exchange timed out")??;
        println!("ECHO_OK");
    }

    tokio::signal::ctrl_c().await?;
    peer.shutdown().await?;
    println!("STOPPED");
    Ok(())
}
```

The protocol reads exactly four bytes, which matches its declared frame bound.
Real protocols should use an equally strict bounded codec rather than reading
unbounded data from a stream.

## 3. Run the first peer

For a short dev test, configure the first process:

```sh
export AUKI_EMAIL='you@example.com'
export AUKI_PASSWORD='...'
export AUKI_DOMAIN_ID='00000000-0000-0000-0000-000000000000'
export AUKI_STATE_DIR='./state-a'
cargo run
```

Replace the Domain UUID with one returned for your account. Startup waits for
the authenticated Domain, current authority, and a confirmed relay route. The
process then prints something like:

```text
READY
  domain: 00000000-0000-0000-0000-000000000000
  peer:   12D3KooW...
  route:  /dns4/relay.dev.aukiverse.com/tcp/443/p2p/12D3KooW.../p2p-circuit/p2p/12D3KooW...
```

Keep it running and copy its complete `peer` and `route` values.

Environment variables are convenient for this experiment, not a production
secret-management strategy.

## 4. Connect a second peer

In another terminal, use a different state directory and the first peer's
printed values:

```sh
export AUKI_EMAIL='you@example.com'
export AUKI_PASSWORD='...'
export AUKI_DOMAIN_ID='00000000-0000-0000-0000-000000000000'
export AUKI_STATE_DIR='./state-b'
export AUKI_REMOTE_PEER_ID='<PEER_ID_PRINTED_BY_PROCESS_A>'
export AUKI_REMOTE_ROUTE='<COMPLETE_ROUTE_PRINTED_BY_PROCESS_A>'
cargo run
```

The second peer receives its own stable identity and relay allocation, then
dials the first peer through the supplied route. A successful authenticated
exchange prints:

```text
ECHO_OK
```

Using the same User account is fine for this experiment. The separate identity
files create two distinct Peer IDs.

Press Ctrl-C in both terminals. Each process awaits ordered shutdown and prints
`STOPPED` after cleaning up its runtime and relay booking.

## Use App credentials instead

A trusted native or headless application can replace the authentication call:

```rust
let auth_session = auth
    .authenticate(Credentials::app(app_access_key, app_secret))
    .await?;
```

Everything after authentication is identical. An App secret must not appear in
a browser, mobile binary, public repository, container image, or log.

## What `AukiPeer` handled

The example did not manually create a `Peer`, `Session`, or `Domain`; schedule
credential refresh; or manage a relay booking. The facade owned:

1. SDK data and session creation.
2. Domain startup and mutual authentication.
3. Verification-key and credential renewal.
4. Relay booking, reservation, and confirmed local route publication.
5. Direct-first route dialing.
6. Status monitoring and authority fencing.
7. Ordered cleanup.

Call `peer.status()` or subscribe with `peer.subscribe_status()` when an
application needs to reflect runtime health in its UI or supervisor.

## Relay is not discovery

The first peer became reachable through a relay, but the second peer still
needed its expected Peer ID and complete route. The SDK does not currently
publish or discover remote routes automatically.

Applications may provide initial route hints with `with_peer_routes`, use
`protocols().open_exact(...)` for one explicit operation, or obtain routes from
their own control plane. Every route remains untrusted until the transport
authenticates the expected Peer ID and Domain.

`known_peers()` reports currently observed authenticated connections. It is not
a directory, route source, or authorization cache.

## Direct-only peers

Use `direct_only()` when the host deliberately does not want a DMS relay
booking:

```rust
let config = AukiPeerConfig::dev("auki-peer-demo", state_dir)
    .direct_only()
    .with_listen_addresses(["/ip4/0.0.0.0/tcp/41001".parse()?])?
    .with_advertised_direct_routes(["/ip4/203.0.113.10/tcp/41001".parse()?])?;
```

Direct-only startup does not prove that the advertised address is reachable
through NAT or a firewall. Replace the documentation-only `203.0.113.10`
address with an address other peers can actually reach. With no listener and
no advertised route the peer is intentionally unreachable, although it may
still dial configured remote peers.

## Lifecycle and deployment

Use one live runtime for each persisted Peer ID. Sequential restart after
`peer.shutdown().await` is supported; simultaneous processes or pods sharing
the identity are not.

Await explicit shutdown when booking deletion matters. Dropping the runtime
fences local work, but it cannot guarantee an external DMS delete; any remaining
booking expires through its authority TTL.

## Machine-managed authority

Robot or Compute applications that receive credentials from another control
plane can use `AukiPeer::start_external`. They provide complete authority
updates through `ExternalAuthorityControl` while the facade continues to own
the networking lifecycle. This advanced path is intended for product adapters,
not ordinary User/App experiments.

## Common failures

- **Identity and credential do not match:** reuse the identity file used for
  the Peer-ID proof; never replace it as an error fallback.
- **Startup cannot confirm a relay:** check DMS/relay availability and that the
  selected principal may request a booking.
- **The remote peer is unreachable:** provide its exact Peer ID and a complete,
  current direct or relay route.
- **The remote rejects the protocol:** confirm that it registered or opted into
  that exact version.
- **A second process fails unexpectedly:** do not share one identity between
  live replicas.

## Where to go next

- Register robot metadata and logs through `peer.peer()` and `peer.session()`.
- Build product protocols through `peer.protocol_context().protocols()`.
- Inspect the facade contract in the
  [`auki-sdk` crate README](../../crates/auki-sdk/README.md).
- Use [`auki-p2p`](../../crates/auki-p2p/README.md) directly only for a custom
  runtime or transport integration.
- Return to the [Auki P2P overview](README.md).
