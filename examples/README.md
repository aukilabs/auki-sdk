# Examples

The examples form one learning path. Pick the smallest one that answers your
question:

| Example | Use it to learn | Runtimes |
| --- | --- | --- |
| [Portable Echo](portable-echo/README.md) | Author one custom protocol in Rust and reuse it everywhere | Rust, Web, Python, Swift |
| [Standard Protocol Playground](standard-protocols/README.md) | Build with Info, Catalog, Registry, Blob, Message, and Stream | Rust, Web, Python, Swift |
| [Camera Mesh](camera-mesh/README.md) | Combine discovery, product authorization, controls, snapshots, and concurrent streams | Rust, Web, Python, Swift |

## Where to start

- **Application developer:** start with
  [Build with an existing protocol](../docs/p2p/getting-started.md), then use the
  Standard Protocol Playground.
- **Protocol author:** copy the Portable Echo shape and follow
  [Author a portable protocol](../docs/p2p/authoring-protocols.md).
- **Demo or interoperability work:** use Camera Mesh after the two smaller
  examples make sense.

Each example keeps protocol framing and validation in Rust. TypeScript, Python,
and Swift provide thin platform adapters plus application UI or lifecycle
wiring.

## Shared assumptions

- Peers must authenticate into the same DDS Domain before they communicate.
- Native and Python processes use separate persistent identity files.
- Browser identities are ephemeral. The Swift examples are also ephemeral but
  show the API an application can use to persist identity bytes.
- App credentials are for trusted native or headless processes only. Browser
  and mobile examples use User login.
- Relay reachability and discovery are separate choices. The examples state
  their defaults explicitly.
- Discovery results and copied routes are hints. Exact protocol operations
  still verify the remote Peer ID and Domain.

## Validation

Every example README separates offline build or unit checks from live
credentialed interoperability. Live tests require a User, an accessible dev
Domain, and the currently deployed DDS, DMS, and relay services.

The protected Web matrices require Playwright Chromium. Swift simulator suites
validate contracts and application logic without credentials; Camera Mesh
publishing and the documented live iOS gates use a physical iPhone.

Return to the [P2P overview](../docs/p2p/README.md) for the architecture and
runtime choices.
