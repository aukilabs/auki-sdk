# Find peers and configure connections

Start with an authenticated peer in the same Domain as the peer you want to
reach. Each call needs the expected remote Peer ID, a compatible route, and the
exact application protocol ID.

## Find a peer through DDS

Enable discovery before starting your peer:

~~~rust
let bootstrap = bootstrap
    .with_dds_tracker(auki_sdk::DdsTrackerMode::DiscoverAndAdvertise);
~~~

Use `DiscoverOnly` when your app should look up peers without publishing itself.
Mount your endpoint after startup; advertisements follow the mounted protocol
IDs and current routes.

Query for the protocol your app understands:

~~~rust
let candidates = peer.discover_protocol("/example/echo/1.0.0").await?;
~~~

Select a candidate through your application's UI or configured expected Peer
ID. Choose a compatible route from `candidate.routes()`, then pass its Peer ID
and that route to your protocol client's exact-route operation.

Discovery is opt-in and results expire. Refresh candidates after a connection
failure. The selected identity is verified during connection; an advertisement
does not grant permission to invoke your application's commands.

## Connect using an address you already have

You may obtain the Peer ID and route from your own configuration or control
plane. Open your protocol directly:

~~~rust
let stream = peer.protocols()
    .open_exact(remote_peer_id, remote_route, "/my-app/ping/1.0.0")
    .await?;
~~~

The returned stream implements `futures::AsyncRead` and `AsyncWrite`; your
protocol owns the conversation and closure. See [custom protocols](protocols.md).

Native Rust can instead configure route hints with
`AukiPeerConfig::with_peer_routes` and call `protocols().open`. DDS lookup
results are not automatically installed as those configured routes.

## Choose reachability

| Need | Configuration |
| --- | --- |
| Accept inbound calls through a relay | Default peer configuration |
| Native outbound calls without a local relay booking | `bootstrap.without_relay()`; no listeners needed |
| Native direct inbound connections | Configure a listener and share a reachable direct address |
| Browser outbound calls only | `AukiPeerReachabilityMode.OutboundOnly` |
| Browser inbound and outbound calls | `AukiPeerReachabilityMode.RelayBacked` |

For a native direct server, pass this configuration to authentication/startup,
replacing the addresses with your server's bind and reachable addresses:

~~~rust
use auki_sdk::AukiPeerConfig;

fn direct_config(dms_base: &str) -> anyhow::Result<AukiPeerConfig> {
    Ok(AukiPeerConfig::new(dms_base)?
        .direct_only()
        .with_listen_addresses(["/ip4/0.0.0.0/tcp/4001".parse()?])?
        .with_advertised_direct_routes(["/dns4/my-server.example/tcp/4001".parse()?])?)
}
~~~

Open the port in your deployment and make the advertised address reachable.
A listener address such as `0.0.0.0` or a dynamically allocated port is not a
public route. To publish through DDS, also enable `DiscoverAndAdvertise`.

Native/Python/Swift peers use TCP routes; browser peers dial WSS relay routes.
A relay-backed peer exposes a TCP/WSS route pair, so both kinds of client can
reach it. An outbound-only browser cannot advertise inbound reachability.

For configurable relay counts and defaults, see the
[reference](../reference/networking.md#configuration).
