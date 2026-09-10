# Auki SDK

Auki SDK lets your app sign in, find peers, and exchange data on the Auki
network. Configure and create an `AukiPeer` inside your existing app to get started.

Use Auki's discovery service or supply peer IDs and addresses yourself. Your
app chooses which protocols to handle.

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
| Look up SDK terminology | [Glossary](docs/reference/glossary.md) |
| Understand the core SDK's direction | [Vision](docs/explanation/vision.md) |
| Build an app with a coding agent | [App-builder skill](docs/skills/auki-sdk-app-builder/SKILL.md) |

## What are you building?

| Build | Use | Start with |
| --- | --- | --- |
| User app, such as Web or iOS | Talk to peers with an email/password or ZITADEL user login | [User authentication](docs/how-to/authenticate.md) |
| Backend service, such as Rust or Python | Talk to peers with an App access key and secret | [Service authentication](docs/how-to/authenticate.md#sign-in-as-a-backend-service) |
| Compute node | Use a Posemesh runner to process eligible DMS tasks across Domains | [Compute requirements](docs/explanation/apps-nodes-and-robots.md#compute-nodes) |
| Robot | Use Posemesh's robot entrypoint to process DMS tasks in its assigned Domain | [Robot requirements](docs/explanation/apps-nodes-and-robots.md#robots) |

Compute nodes use a signing wallet and follow DDS staking requirements.
SDK apps, services, and robots do not require a node wallet or stake.
See [apps, services, compute nodes, and robots](docs/explanation/apps-nodes-and-robots.md)
for authentication and Domain restrictions.

Use P2P for data exchange. Submit tasks through **DMS**, the source of truth
for task state; Posemesh runners handle execution.

## Repository

| Directory | Contents |
| --- | --- |
| [`core/`](core/) | Stable crates: `auki-sdk`, `auki-p2p`, `auki-auth`, `auki-relay-booking`, `auki-dms` |
| [`core/bindings/`](core/bindings/) / [`core/examples/`](core/examples/README.md) | SDK bindings and networking examples |
| [`labs/`](labs/) | Experimental crates, including the optional [`auki-protocols`](labs/auki-protocols/README.md) |
| [`labs/bindings/`](labs/bindings/) / [`labs/examples/`](labs/examples/) | Experimental bindings and examples |

To work on the SDK, see [Contributing](CONTRIBUTING.md).

[MIT license](LICENSE).
