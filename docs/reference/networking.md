# Networking reference

For HTTP data APIs and managed task execution, see the
[Domain data](domain-data.md) and [task runtime](tasks.md) references.

## Platforms and installation

These instructions build from source. Use the same SDK revision for your app,
protocol crates, and bindings.

| Platform | Peer API | Peer ID across restarts | Custom protocols |
| --- | --- | --- | --- |
| Rust | `AukiPeerBootstrap`, `AukiPeer` | Saved file or new key | Rust handler |
| Python | `auki_sdk.AukiSession`, `AukiPeer` | Saved file | Rust adapter in the same Python extension |
| Web | `AukiUserSession`, `AukiPeer` | New key | Rust adapter in the same Wasm module |
| iOS and Android | `AukiSession`, `AukiPeer` | App can save identity bytes | Rust adapter in the same UniFFI library |
| Expo Web/iOS/Android | `@aukilabs/auki-sdk-expo` | Managed by the platform bridge | Use operations exported by the bridge |

All supported platforms expose User-password login. Trusted Rust and Python
services can use App access keys and secrets. Rust, Web, iOS, Android, and Expo
also support importing a ZITADEL session.

iOS and Android share one UniFFI crate, [`auki-sdk-uniffi`](../../core/bindings/uniffi/auki-sdk-uniffi/README.md).
Its library is `libauki_sdk_uniffi`: Swift on iOS, Kotlin on Android. Web is a
separate crate, [`auki-sdk-web`](../../core/bindings/web/auki-sdk-web/README.md),
because that binding is Wasm rather than UniFFI.

### Rust

Requires Rust 1.89 or newer. For an app next to this checkout, add to `Cargo.toml`:

~~~toml
[dependencies]
auki-sdk = { path = "../auki-sdk/core/auki-sdk" }
anyhow = "1"
tokio = { version = "1", features = ["macros", "rt-multi-thread", "signal"] }
~~~

You can also use a Git dependency with the repository URL and an exact `rev`.
The [tutorial](../tutorials/first-peer.md) runs directly from this checkout.

Generate the Rust API reference from the repository root:

~~~sh
cargo doc --locked -p auki-sdk -p auki-auth -p auki-p2p -p auki-dms -p auki-domain-client -p auki-tasks --no-deps
~~~

Open `target/doc/auki_sdk/index.html`.

### Other platforms

| Platform | Build requirements and instructions |
| --- | --- |
| Python | Python 3.8+, Rust, Maturin; [build the binding](../../core/bindings/python/auki-sdk-py/README.md) |
| Web | Rust, `wasm32-unknown-unknown`, wasm-pack 0.13.1, Node 20.19+ on 20.x or 22.12+; [Wasm binding](../../core/bindings/web/auki-sdk-web/README.md) |
| iOS and Android | Rust and the [UniFFI facade](../../core/bindings/uniffi/auki-sdk-uniffi/README.md). iOS also needs Swift 6, Xcode, and iOS 17+. Android also needs NDK r27+ and JDK 17 or 21 |
| Expo | Those toolchains and Expo; [build the package](../../core/bindings/expo/README.md). Web uses the Wasm package, iOS and Android use `libauki_sdk_uniffi` |

## Public Rust entry points

| API | Use it to… |
| --- | --- |
| `AukiPeerBootstrap` | Sign in and start a peer in a selected Domain |
| `AuthClient`, `AuthSession` | Set service URLs or reuse a login session |
| `AukiPeerConfig` | Configure relays, listeners, addresses, and discovery |
| `AukiPeer::protocols()` | Register a handler or open an outgoing stream |
| `AukiProtocolSpec` | Set protocol ID, concurrency, and declared frame-size limit |
| `AukiProtocolRegistration::close()` | Stop accepting requests and wait for active handlers |
| `discover()`, `discover_protocol()` | Look up peers and their advertised addresses |
| `resolve(identity, protocol_id)` | Find every advertised peer for an exact Peer, Robot or Compute identity |
| `protocols().open_resolved(&result)` | Open a selected result and verify its signed identity |
| `protocol_context().routes()` | Read your peer's addresses and watch for changes |
| `known_peers()` | Inspect authenticated connections (native) |
| `status()`, `subscribe_status()` | Check native peer readiness and watch for changes |
| `wait_stopped()`, `lifecycle()` | Wait for the peer to stop |
| `shutdown()` | Release relay bookings and close connections |
| `AukiPeer::start_external` | Start a native peer using authentication managed by your host |

Native `protocols().open` uses addresses configured with
`AukiPeerConfig::with_peer_routes`. DDS lookup does not add addresses there;
pass a selected identity lookup result to `open_resolved`, or an independently
obtained address to `open_exact`. See [identity resolution](peer-resolution.md)
for the required DDS rollout, multiple-peer selection, and completeness limits.

`start_external(identity, update, config)` returns a peer and an authentication
control handle. Respond to `next_refresh_request` and pass complete
`ExternalAuthorityUpdate` values to `replace`. Python exposes the same
`next_refresh_request` method. `replace` keeps the peer and its relay
reservation; applications still reopen *application* streams if a hop dies.
There is no public reconnect API. For managed compute and robot peers, the
[task runtime](tasks.md#authority-and-peer-lifetime) handles renewal.

See the [public exports](../../core/auki-sdk/src/lib.rs) for platform availability.

## Configuration

These values describe `AukiPeerConfig` and protocol registration.

| Setting | Default / constraint |
| --- | --- |
| Relay | Enabled; public pool, one provider |
| Relay count | Native: 1–3; browser: one |
| Booking duration | 86,400 seconds; range 300–86,400 whole seconds |
| Status polling | 30 seconds; range 1–60 whole seconds |
| DDS discovery | Disabled |
| Discovery modes | `DiscoverOnly` or `DiscoverAndAdvertise` |
| Native listeners | None |
| Advertised direct addresses | None; require reachable, nonzero ports |
| Configured remote addresses | None; native `with_peer_routes` sets them |
| Local address capacity | 16 entries; a relay's TCP/WSS pair uses one entry |
| App protocol ID | At most 255 bytes; use your own namespace, e.g. `/my-app/ping/1.0.0` |
| Concurrent incoming streams | Set per protocol, 1–1,024 |
| Declared frame-size limit | 1 byte–64 MiB; your decoding code must enforce it |

The `/auki/` protocol namespace is reserved. Source:
[peer configuration](../../core/auki-sdk/src/config.rs) and
[protocol constraints](../../core/auki-p2p/src/application_protocol.rs).

`AuthEnvironment::dev()` and `AukiPeerConfig::dev()` select shared development
services. You can set API, DDS, and DMS URLs for your own environment. Public
service URLs require HTTPS; limited loopback HTTP configurations are supported
for local development.

## Errors and recovery

| Symptom | Action |
| --- | --- |
| Peer authorization fails | Check credentials, Domain access, and service URLs |
| Discovery is disabled | Enable it before starting the peer |
| No matching peers | Check the other peer's Domain, protocol ID, registered handler, and advertising mode; retry the lookup |
| `NoRoutes` | Set native remote addresses or use `open_exact` with an address |
| Invalid address / all addresses fail | Verify the Peer ID, current address, TCP/WSS support, and protocol ID; check that the peer is reachable |
| Duplicate protocol | Close the previous registration before registering the same ID |
| `AuthorityUnavailable` / `RelayUnavailable` | Pause new network requests and watch status for recovery or failure |
| `RelayReservationClosed` while reopening through a previously reserved relay | Wait for the source reservation to be confirmed again before retrying; a fresh destination address alone does not establish source readiness |
| `Failed`, `Stopping`, `Stopped` | Stop sending requests and finish cleanup; inspect the stop result |
| ZITADEL `authentication_required` | Sign in again |
| ZITADEL `configuration` / `authorization_denied` | Check session/service settings and access to the selected Domain |
| ZITADEL `persistence` / `transient` | Retry using the same session |
| ZITADEL `closed` / `cancelled` | Stop using the closed session or handle the cancelled operation |

Finding a peer does not grant it permission to call your app's operations.
Check the authenticated peer in your handler.

Native and browser transports fence new source circuits during reservation
recovery, including after local teardown completes. The fence prevents a newly
opened connection from using old provider authority that is about to be revoked.
It does not replay interrupted data or guarantee that an existing stream survives
connection loss. Applications requiring complete delivery still need acknowledgements
and replay/deduplication.
