# Use a custom application protocol

Both peers must agree on a protocol ID, message format, and response. Define
these in your app; `auki-protocols` is optional.

## Start with Echo

Use [Portable Echo](../../core/examples/portable-echo/README.md) as a template:

| Part | Format |
| --- | --- |
| Protocol ID | `/example/echo/1.0.0` |
| Request | Four-byte unsigned big-endian payload length, then 1–1,024 payload bytes |
| Response | The same length and payload, then the stream closes |

Add the [Echo dependency](connect.md). This function serves requests on a
running native peer until Ctrl-C or a peer failure:

~~~rust
use auki_portable_echo::EchoEndpoint;
use auki_sdk::AukiPeer;

async fn serve_echo(peer: &AukiPeer) -> anyhow::Result<()> {
    let endpoint = EchoEndpoint::mount(peer.protocols())?;
    let result = tokio::select! {
        signal = tokio::signal::ctrl_c() => signal.map_err(anyhow::Error::from),
        stopped = peer.wait_stopped() => Err(anyhow::anyhow!("Peer stopped: {stopped:?}")),
    };
    let cleanup = endpoint.close().await;
    result?;
    cleanup?;
    Ok(())
}
~~~

Use [`EchoClient::send_exact`](connect.md#use-your-own-discovery-or-a-known-address)
from the other peer to get the same bytes back. The client accepts no incoming
requests. After serving, [shut down the peer](lifecycle.md#close-your-handlers-then-stop-the-peer),
including on failure.

## Replace Echo with your messages

In your own Rust crate, adapt:

1. [wire.rs](../../core/examples/portable-echo/src/wire.rs): change the protocol
   ID, request/response types, and the functions that encode and decode them.
2. [endpoint.rs](../../core/examples/portable-echo/src/endpoint.rs): register
   your handler and open outgoing streams. Keep the timeouts and cleanup.

Register with `peer.protocols().register(spec, handler)`. For example,
`AukiProtocolSpec::new("/my-app/ping/1.0.0", 8, 1024)` allows eight concurrent
incoming streams and declares a 1,024-byte frame limit. **Your decoding code
must enforce this limit** before allocating an incoming payload.

The handler receives an `AukiProtocolStream` and returns a future yielding `()`.
Check `stream.remote_peer()` against your app's permissions before handling
a request. Keep the registration alive while serving; await
`registration.close()` when finished.

Call `peer.protocols().open_exact(peer_id, address, protocol_id)` to open an
outgoing stream with `futures::AsyncRead` and `AsyncWrite`. Set timeouts for
opening, reading/writing, and closing; clean up after errors too. Change the
protocol ID for incompatible changes; the SDK does not choose an older version.

## Call it from Python, JavaScript, or Swift

Compile your Rust adapter with the SDK in the same Python extension, Wasm
module, or Swift framework. Peer handles cannot cross separately compiled
copies of the SDK.

Copy the structure of the Echo [Python](../../core/examples/portable-echo/python/src/lib.rs),
[Web](../../core/examples/portable-echo/web/src/lib.rs), or
[Swift](../../core/examples/portable-echo/swift/ffi/src/lib.rs) adapter. Code shared
with browsers needs portable async I/O and timers, as used in Echo; the `tokio`
example above is native only.

The [`auki-protocols`](../../labs/auki-protocols/README.md) implementations are
experimental, including those bundled by `standard-protocols` binding features.
