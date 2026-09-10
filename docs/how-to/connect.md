# Connect to a peer

Start a peer in the same Domain as the app you want to reach. You need its
Peer ID, a reachable address, and a protocol that both apps implement.

The Rust examples below call the [Echo app](../tutorials/first-peer.md).
Alongside the [SDK dependencies](../reference/networking.md#rust), add:

~~~toml
auki-portable-echo = { path = "../auki-sdk/core/examples/portable-echo" }
~~~

## Use your own discovery or a known address

Get the Peer ID and address from your own discovery service, configuration, or
the other app. This function sends `hello` and returns the echoed bytes:

~~~rust
use auki_portable_echo::EchoClient;
use auki_sdk::AukiPeer;

async fn send_hello(peer: &AukiPeer, peer_id: &str, address: &str) -> anyhow::Result<Vec<u8>> {
    let receipt = EchoClient::new(peer.protocols())
        .send_exact(peer_id.parse()?, address.parse()?, b"hello".to_vec())
        .await?;
    Ok(receipt.payload)
}
~~~

Pass the full Peer ID and `route:` value printed by the native Echo app.
The SDK verifies the connected peer's identity. No DDS lookup is needed.
For your own protocol, use `peer.protocols().open_exact` and read and write
the returned stream; see [custom protocols](protocols.md).

## Use Auki discovery

Enable DDS discovery on your bootstrap before starting the peer:

~~~rust
let bootstrap = bootstrap.with_dds_tracker(auki_sdk::DdsTrackerMode::DiscoverOnly);
~~~

On the receiving app, use `DiscoverAndAdvertise` and register an incoming
protocol handler. DDS will advertise its protocol IDs and reachable addresses.

This native Rust function finds an expected Echo peer and tries its TCP
addresses. Supply a Peer ID chosen by the user or your app's configuration:

~~~rust
use anyhow::Context;
use auki_portable_echo::{EchoClient, PROTOCOL_ID};
use auki_sdk::{AukiPeer, PeerId};

async fn send_discovered(peer: &AukiPeer, expected: PeerId) -> anyhow::Result<Vec<u8>> {
    let candidate = peer.discover_protocol(PROTOCOL_ID).await?
        .into_iter()
        .find(|candidate| candidate.peer_id() == expected)
        .context("Echo peer not found; check that it is running and advertising")?;

    let client = EchoClient::new(peer.protocols());
    for route in candidate.routes() {
        if route.to_string().split('/').any(|part| part == "wss") {
            continue; // Browsers use WSS; this native example uses TCP.
        }
        match client.send_exact(expected, route.clone(), b"hello".to_vec()).await {
            Ok(receipt) => return Ok(receipt.payload),
            Err(error) => eprintln!("{route}: {error}"),
        }
    }
    anyhow::bail!("No working TCP address; refresh discovery and try again")
}
~~~

Advertisements expire. If the peer is missing or its addresses fail, check
that it is still advertising and run a fresh lookup. The Echo client limits
each connection, exchange, and stream close to five seconds.

Discovery tells you where a peer may be; your handler must still check what
that peer is allowed to do.

## Accept connections directly or through a relay

| Need | Configuration |
| --- | --- |
| Accept connections through a relay | Default peer configuration |
| Native outbound connections only | `bootstrap.without_relay()` |
| Native direct inbound connections | Configure a listener and publish a reachable address |
| Browser outbound connections only | `AukiPeerReachabilityMode.OutboundOnly` |
| Browser inbound and outbound connections | `AukiPeerReachabilityMode.RelayBacked` |

For a native server with a public TCP port, use this configuration when
[authenticating](authenticate.md). Replace the hostname and port with yours:

~~~rust
use auki_sdk::AukiPeerConfig;

fn direct_config(dms_base: &str) -> anyhow::Result<AukiPeerConfig> {
    Ok(AukiPeerConfig::new(dms_base)?
        .direct_only()
        .with_listen_addresses(["/ip4/0.0.0.0/tcp/4001".parse()?])?
        .with_advertised_direct_routes(["/dns4/my-server.example/tcp/4001".parse()?])?)
}
~~~

Open the port in your firewall. Share the advertised address, not the
`0.0.0.0` listener address. Enable `DiscoverAndAdvertise` to publish it in DDS.

Native, Python, and Swift peers use TCP. Browsers need WSS relay addresses,
so use a relay to let them reach your server. Each relay provides a TCP/WSS
address pair. See [configuration](../reference/networking.md#configuration)
for relay settings and native `with_peer_routes` defaults.
