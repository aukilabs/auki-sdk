# Auki SDK

Auki SDK lets your app sign in, find peers, and exchange data on the Auki
network. Create an `AukiPeer` inside your existing app; no separate process
is needed.

Use Auki's discovery service or supply peer IDs and addresses yourself. Your
app chooses its message format and handles incoming requests.

## Start here

[Run two peers and send a message](docs/tutorials/first-peer.md) with the Rust
Echo example. For Python, Web, Swift, or Expo, see
[installation](docs/reference/networking.md#platforms-and-installation).

## Documentation

| I want to… | Read |
| --- | --- |
| Sign in and choose a Domain | [Authenticate](docs/how-to/authenticate.md) |
| Use discovery or connect to a known address | [Connect to a peer](docs/how-to/connect.md) |
| Exchange my own messages | [Use a custom protocol](docs/how-to/protocols.md) |
| Keep a Peer ID and stop cleanly | [Manage a peer](docs/how-to/lifecycle.md) |
| Understand peers, Domains, and task handling | [How networking works](docs/explanation/networking.md) |
| Look up APIs, defaults, or errors | [Networking reference](docs/reference/networking.md) |

## Robot and compute runners

Submit robot and compute tasks through **DMS**, the source of truth for task
state. [Posemesh runners](https://github.com/aukilabs/posemesh/tree/main/core/compute-node)
handle execution. Use P2P for data exchange; do not dispatch tasks over it.
A robot runner must execute one task at a time and reject new tasks while busy.

These are application rules. The SDK transports your bytes and does not enforce
task scheduling.

## Repository

| Directory | Contents |
| --- | --- |
| [`core/`](core/) | Stable crates: `auki-sdk`, `auki-p2p`, `auki-auth`, `auki-relay-booking`, `auki-dms` |
| [`core/bindings/`](core/bindings/) / [`core/examples/`](core/examples/README.md) | SDK bindings and networking examples |
| [`labs/`](labs/) | Experimental crates, including the optional [`auki-protocols`](labs/auki-protocols/README.md) |
| [`labs/bindings/`](labs/bindings/) / [`labs/examples/`](labs/examples/) | Experimental bindings and examples |

[MIT license](LICENSE).
