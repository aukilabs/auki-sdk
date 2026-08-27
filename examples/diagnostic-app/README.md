# Authenticated Domain diagnostic

This example is a small command-line proof of the native Stage 1 `Domain` API.
One process owns one stable identity and one DDS Domain UUID. The host supplies
the DDS ES256 verification key, the peer-bound signed access token, listener
addresses, and explicit peer routes.

The app does not contact DDS or DMS, discover peers, book relays, elect a
manager, synchronize a Domain clock, or broadcast diagnostics. Those are not
hidden startup steps.

## Build and inspect the CLI

From the SDK root:

```sh
cargo build --locked -p auki-diagnostic-app
cargo run --locked -p auki-diagnostic-app -- --help
```

The `run` command accepts:

- `--domain`: the exact DDS Domain UUID;
- `--identity`: canonical protobuf-encoded Ed25519 private-key bytes;
- `--dds-public-key`: the current DDS ES256 public key PEM;
- `--credential`: the compact signed P2P access JWT bound to the identity's
  Peer ID and Domain UUID;
- one or more `--listen` multiaddrs;
- zero or more `--route PEER_ID=MULTIADDR` dial hints; and
- optional `--fetch-peer` targets for resource catalog v0.2 requests.

`peer-id --identity FILE` prints the stable Peer ID a host must use when it
requests the credential.

## Complete local proof

Run the checked script from the SDK root:

```sh
./examples/diagnostic-app/scripts/local-demo.sh
```

It builds the CLI, creates two explicitly insecure short-lived demo
credentials, starts two separate processes on direct TCP listeners, and makes
each process fetch the other's catalog. It then proves malformed, wrong-Domain,
and wrong-Peer tokens are rejected before any `CATALOG` output. Ports can be
overridden with
`AUKI_DIAGNOSTIC_PORT_A` and `AUKI_DIAGNOSTIC_PORT_B`.

Successful process output is deliberately scriptable:

```text
READY domain_id=... peer_id=... status=Ready listeners=...
CATALOG peer_id=... count=1 resource_ids=...
PEERS count=1 peer_ids=...
LEFT peer_id=...
```

An invalid, wrong-Peer, or wrong-Domain credential emits `JOIN_FAILED`, exits
non-zero, and never emits a remote `CATALOG` line.

`demo-material` generates a throwaway DDS signing key in memory and only writes
its public key plus short-lived credentials. Its output is only for loopback
testing and expires after 30 minutes. Production hosts must supply stable
identity and authority files from their own secure integration.

## Route ownership

Routes are hints, not authorization. Today they can come from static
configuration or the embedding application. DDS/DMS adapters or a future
discovery source can call the same Domain route surface without changing the
authentication rule. Relay assignment and booking remain outside this CLI and
the public `Domain` facade. In Posemesh, DMS/compute-node owns booking and
constructs the `RelayProvider`; the lower-level node reserves it, then only the
confirmed publishable relay route is distributed and installed as an explicit
Domain route.
