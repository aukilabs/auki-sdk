# How Auki networking works

`AukiPeer` runs inside your app and handles network connections,
authentication renewal, and relay bookings.

## Peers and Domains

A **Peer ID** comes from a network keypair. Saving the key keeps the ID across
restarts. Apps can share a User account, but each running peer needs its own key.

Peers communicate within a selected **Domain**. Your account needs access to
that Domain. The SDK verifies the other peer's ID and Domain membership;
your app decides which requests to allow.

## Discovery and connections

Get a peer's ID and address from your own discovery service, configuration,
or Auki's DDS service. DDS discovery is off by default.

An address is called a **route** in the API. Native peers use TCP, directly or
through a relay; browsers use WSS relay addresses. A relay accepts connections
on your behalf, so you do not need a public port. The SDK books it through DMS.

Relay use and discovery are separate choices. You can use a relay and share
its address yourself.

## Messages and protocols

Apps agree on a protocol ID and message format. The SDK opens an authenticated
stream; your code reads and writes the bytes. You register the handlers to serve.

You can implement your own protocol. The implementations in
[`auki-protocols`](../../labs/auki-protocols/README.md) are optional and experimental.

## Robot and compute tasks

Tasks go through **DMS**, the source of truth for task state.
[Posemesh runners](https://github.com/aukilabs/posemesh/tree/main/core/compute-node)
handle orchestration, execution, heartbeats, and results.

Use P2P for data exchange, never task dispatch. For robots, the runner must
execute one task at a time and reject new tasks while busy.

Applications and runners enforce these rules. The SDK does not inspect
message contents or schedule tasks.
