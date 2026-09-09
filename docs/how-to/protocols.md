# Use a custom application protocol

Choose an existing product protocol or define one in your own crate. Both ends
must implement the same protocol ID and conversation. The networking engine
does not require anything from `auki-protocols`.

## Use an existing endpoint

The [Portable Echo application](../../core/examples/portable-echo/native/src/main.rs)
shows the complete lifecycle. Its public application API is small:

~~~rust
let endpoint = auki_portable_echo::EchoEndpoint::mount(peer.protocols())?;
let result = endpoint.send_exact(remote_peer_id, remote_route, b"hello").await;
let cleanup = endpoint.close().await;
let receipt = result?;
cleanup?;
~~~

Keep the endpoint alive for as long as your app should accept requests. For
outbound calls only, use its `EchoClient`; constructing a client does not mount
an inbound handler.

## Define your own contract

Use [Portable Echo's wire module](../../core/examples/portable-echo/src/wire.rs) as a
small reference. Define these in your product crate:

1. An exact ID in your namespace, such as `/my-app/ping/1.0.0`.
2. Request/response types, framing, and message-size limits.
3. The conversation: who writes first, when a response ends, and how errors
   and cancellation are handled.
4. Deadlines and the permissions required for each operation.

Keep your codec independent of the host language and transport. Reject an
oversized length before allocating its payload. Change the protocol ID when
the wire format or conversation becomes incompatible.

## Register a handler

After implementing your async `handle_request` function, mount it on the peer:

~~~rust
let registration = peer.protocols().register(
    auki_sdk::AukiProtocolSpec::new("/my-app/ping/1.0.0", 8, 1024)?,
    handle_request,
)?;
~~~

The handler receives an `AukiProtocolStream` and returns a future yielding
`()`. Inspect `stream.remote_peer()` and apply your product's permissions
before performing an operation. The stream is already authenticated to this
Domain.

The spec above permits eight concurrent inbound streams and declares a
1,024-byte frame limit. **Your codec must enforce the frame limit.** The engine
does not interpret your bytes. Bound the conversation's duration and attempt
stream cleanup on success and failure.

Keep `registration` alive while serving; await `registration.close()` to stop.
For a complete implementation with deadlines and cleanup, use the
[Echo endpoint](../../core/examples/portable-echo/src/endpoint.rs).

## Call the protocol

Use `peer.protocols().open_exact(remote_peer_id, route, protocol_id)`, then run
your client codec on the returned stream. Apply deadlines to opening, exchange,
and closure. Handle unsupported-protocol errors explicitly; the SDK does not
choose an older application version for you.

## Use the protocol from another language

Share the Rust codec and endpoint. Add a thin adapter to the same Wasm module,
Python extension, or Swift framework that owns the peer. Runtime handles from
separate compiled artifacts are not interchangeable.

The [Web](../../core/examples/portable-echo/web/src/lib.rs),
[Python](../../core/examples/portable-echo/python/src/lib.rs), and
[Swift](../../core/examples/portable-echo/swift/ffi/src/lib.rs) Echo adapters demonstrate
this boundary. Use portable async I/O and timers in code shared with the browser.

The current `auki-protocols` implementations remain experimental, even where
bindings expose features named `standard-protocols`. Enabling a feature or
importing a type does not mount an endpoint.
