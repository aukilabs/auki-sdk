# Auki SDK

Build applications that authenticate, find peers, and exchange data over direct
or relay connections. `AukiPeer` owns the networking runtime: identity binding,
renewable authority, transport, reachability, and shutdown.

Your application chooses what to send and which application protocols to
implement. Starting a peer mounts no application protocol.

The current protocols in [`auki-protocols`](labs/auki-protocols/README.md) are
**optional and experimental**, so the crate lives in `labs/` for now. It remains
the intended home for protocols we select and freeze as stable.

| Directory | Contents |
| --- | --- |
| [`core/`](core/) | Stable SDK crates: `auki-sdk`, `auki-p2p`, `auki-auth`, `auki-relay-booking`, and `auki-dms` |
| [`core/bindings/`](core/bindings/) | SDK bindings for Python, Web, Swift, and Expo |
| [`core/examples/`](core/examples/README.md) | Networking applications; start with portable echo |
| [`labs/`](labs/) | Experimental crates, including `auki-protocols`, `auki-identity`, and `auki-hash` |
| [`labs/bindings/`](labs/bindings/) | Bindings for experimental crates |
| [`labs/examples/`](labs/examples/) | Experimental protocol and application examples |

Some SDK binding builds include experimental protocol features. Applications
still choose which endpoints to mount.

## Start here

[Connect two peers and exchange a message](docs/tutorials/first-peer.md).

Use one reviewed source revision for your app and its bindings.
See [platform support and installation](docs/reference/networking.md#platforms-and-installation)
for Rust, Python, Web, Swift, and Expo.

## Documentation

| I want to… | Read |
| --- | --- |
| Learn by running an app | [Connect two peers](docs/tutorials/first-peer.md) |
| Sign in and choose a Domain | [Authenticate](docs/how-to/authenticate.md) |
| Find a peer or configure reachability | [Connect](docs/how-to/connect.md) |
| Exchange my own messages | [Use a custom protocol](docs/how-to/protocols.md) |
| Preserve identity and handle shutdown | [Manage a peer](docs/how-to/lifecycle.md) |
| Understand the moving parts | [How networking works](docs/explanation/networking.md) |
| Look up APIs, defaults, or errors | [Networking reference](docs/reference/networking.md) |

## Robot and compute runners

[Posemesh](https://github.com/aukilabs/posemesh/tree/main/core/compute-node)
builds on this SDK to run robot and compute tasks. Runner behavior, task
scheduling, heartbeats, and product permissions belong to that layer.

[MIT license](LICENSE).
