# Networking reference

## Platforms and installation

Use a reviewed SDK source revision consistently across your application,
protocol crate, and bindings. The packages' `0.1.0` source version does not
establish package-registry availability or make experimental protocols stable.

| Platform | Peer API | Identity | Application protocols |
| --- | --- | --- | --- |
| Rust | `AukiPeerBootstrap`, `AukiPeer` | Persistent or ephemeral | Native Rust handlers; shared Rust can target Wasm |
| Python | `auki_sdk.AukiSession`, `AukiPeer` | Persistent file | Rust adapter in the same Python extension |
| Web | `AukiUserSession`, `AukiPeer` | Ephemeral | Rust adapter in the same Wasm module |
| Swift/iOS | `AukiUserSession`, `AukiPeer` | Ephemeral or app-persisted | Rust adapter in the same framework |
| Expo Web/iOS | `@aukilabs/auki-sdk-expo` session/peer handles | Managed by the platform bridge | Only exported bridge operations |
| Expo Android | Unimplemented | — | — |

User-password authentication is exposed on the supported platforms. App
access-key/secret authentication is for trusted native Rust/Python hosts.
ZITADEL session import is exposed in Rust, Web, Swift, and Expo Web/iOS.

### Rust

Requires Rust 1.89 or newer. For an app next to this checkout, use:

~~~toml
[dependencies]
auki-sdk = { path = "../auki-sdk/core/auki-sdk" }
anyhow = "1"
tokio = { version = "1", features = ["macros", "rt-multi-thread", "signal"] }
~~~

A downstream Git dependency can use the repository URL with an exact reviewed
`rev`. The [tutorial](../tutorials/first-peer.md) runs directly from this
workspace.

Generate the public Rust API reference locally:

~~~sh
cargo doc --locked -p auki-sdk -p auki-auth -p auki-p2p --no-deps
~~~

The entry point is `target/doc/auki_sdk/index.html`.

### Other hosts

| Host | Source setup |
| --- | --- |
| Python | Python 3.8+, Rust, and Maturin; [build the binding](../../core/bindings/python/auki-sdk-py/README.md) |
| Web | Rust, `wasm32-unknown-unknown`, wasm-pack 0.13.1, Node 20.19+ on 20.x or 22.12+; [run the Web host](../../core/examples/portable-echo/web/README.md) |
| Swift | Swift 6, Xcode, iOS 17+, and Apple Rust targets; [build the Swift host](../../core/examples/portable-echo/swift/README.md) |
| Expo | Supported Web/iOS toolchains above and Expo; [package setup](../../core/bindings/expo/README.md) |

## Public Rust entry points

| API | Purpose |
| --- | --- |
| `AukiPeerBootstrap` | Authenticate, select a Domain, and start an identity |
| `AuthClient`, `AuthSession` | Explicit service environments and imported login sessions |
| `AukiPeerConfig` | Reachability, initial routes, and discovery settings |
| `AukiPeer::protocols()` | Register or open an application protocol |
| `AukiProtocolSpec` | Exact protocol ID and inbound handler bounds |
| `AukiProtocolRegistration::close()` | Stop a mounted handler |
| `discover()`, `discover_protocol()` | Fresh untrusted discovery candidates |
| `protocol_context().routes()` | Confirmed local routes and route changes |
| `known_peers()` | Native observations of authenticated connections |
| `status()`, `subscribe_status()` | Native readiness |
| `wait_stopped()`, `lifecycle()` | Terminal lifecycle observation |
| `shutdown()` | Await network cleanup |
| `AukiPeer::start_external` | Native startup with host-managed machine authority |

The [public exports](../../core/auki-sdk/src/lib.rs) define the exact target
availability. Low-level transport APIs live in `auki-p2p`.

## Configuration

Values below describe `AukiPeerConfig`, not example environment variables.

| Setting | Default / constraint |
| --- | --- |
| Relay | Enabled; public pool, one provider |
| Relay count | Native: 1–3; browser: one |
| Booking duration | 86,400 seconds; accepted range 300–86,400 whole seconds |
| Status polling | 30 seconds; accepted range 1–60 whole seconds |
| DDS discovery | Disabled |
| Discovery modes | `DiscoverOnly` or `DiscoverAndAdvertise` |
| Native listeners | None |
| Native advertised direct routes | None; configure reachable, nonzero ports |
| Initial remote routes | None; native `with_peer_routes` configures them |
| Local route slots | 16; a relay provider consumes one slot for its TCP/WSS pair |
| App protocol ID | At most 255 bytes; use your own namespace, e.g. `/my-app/ping/1.0.0` |
| Inbound concurrency | Declared per protocol, 1–1,024 |
| Declared frame bound | 1 byte–64 MiB; enforced by the application's codec |

Source: [peer configuration](../../core/auki-sdk/src/config.rs) and
[protocol constraints](../../core/auki-p2p/src/application_protocol.rs).
The `/auki/` protocol namespace is reserved.

`AuthEnvironment::dev()` and `AukiPeerConfig::dev()` select shared development
services. Explicit API, DDS, and DMS bases are configurable; public endpoints
require HTTPS. The SDK permits limited loopback HTTP configurations for local
development.

## Errors and recovery

| Symptom | Check or action |
| --- | --- |
| Peer authorization fails | Credentials, selected Domain access, and service environment |
| Discovery is disabled | Enable the tracker before startup |
| No matching peers | Same Domain, exact protocol ID, mounted endpoint, and advertising mode; retry a fresh lookup |
| `NoRoutes` | Configure native route hints or use `open_exact` with a selected route |
| Invalid route / all routes fail | Expected Peer ID, current address, compatible TCP/WSS transport, reachability, and protocol support |
| Duplicate protocol | Retain one registration for an exact ID; close it before replacing |
| `AuthorityUnavailable` / `RelayUnavailable` | Pause new work and observe status for recovery or terminal failure |
| `Failed`, `Stopping`, `Stopped` | Stop submitting work; complete cleanup and handle the terminal result |
| ZITADEL `authentication_required` | Sign in again |
| ZITADEL `configuration` / `authorization_denied` | Fix session/service configuration or select a readable Domain |
| ZITADEL `persistence` / `transient` | Retry on the retained session |
| ZITADEL `closed` / `cancelled` | Handle the operation/session lifecycle termination |

Discovery candidates, routes, and `known_peers()` are not authorization lists.
Use authenticated peer information inside your protocol's permission checks.
