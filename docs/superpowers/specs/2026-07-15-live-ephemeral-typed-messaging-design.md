# Live Ephemeral Typed Messaging Design

**Status:** Approved and implemented for issue #311.

## Decision

The SDK provides one receiver-owned, live-only typed-message channel. A message
contains an opaque type string, one `timestamp_ns`, and opaque payload bytes.
The receiver gets the channel Resource identity and the Noise-authenticated
sender `PeerId` with those unchanged fields.

This branch replaces the earlier unmerged control-specific direction with a
small transport primitive. The SDK routes bytes; it does not define application
commands, state transitions, acknowledgements beyond transport acceptance, or
device behavior.

## Domain API

Applications declare channels before joining:

```rust
let mut domain = DomainBuilder::new(&peer, &session, config)
    .message_channel(
        MessageChannelResource {
            owner_peer_id: identity.peer_id(),
            resource_id: "application-events".into(),
            clock: session.monotonic_clock(),
        },
        32,
    )?
    .join()
    .await?;

let mut messages = domain
    .take_message_channel_receiver("application-events")
    .expect("declared channel");
```

`DomainBuilder` rejects a row whose owner differs from the `Peer` used to join,
a duplicate owner/resource-id pair, an invalid row, zero receiver capacity, or
a clock `RegistryRef` that does not exactly match a clock registered in the
supplied `Session` by peer/id/hash. Before any bootstrap or Discovery I/O, join
also requires the supplied `Peer`, originating `Session`, configured
`PeerIdentity`, and pre-built swarm local peer id to be identical.
When join succeeds, each row, bounded receiver, and `NetworkRuntime`
registration are bound together. Dropping the receiver deregisters the row and
closes active senders. Runtime shutdown drains every registration, so a
receiver retained outside a clean Domain leave or Domain drop closes.

Consumers explicitly fetch Resource Catalog v0.3, choose a discovered
`message_channel` row, and open it against the peer that served it:

```rust
let catalog = domain.fetch_resources_catalog_v3(remote_peer).await?;
let sender = domain.open_message_channel(remote_peer, &channel).await?;
sender
    .send(
        "ambientmovement.command.v1",
        9_876_543_210,
        "opaque application payload: åuki".as_bytes(),
    )
    .await?;
```

`open_message_channel` rejects `channel.owner_peer_id != remote_peer`. The
returned `MessageChannelSender` supports multiple types over one persistent
substream. `Domain::send_message` is the open/send-once convenience form.
Neither API exposes history modes because the protocol has none.

`MessageChannelReceiver::recv` returns:

- the exact `MessageChannelResource`;
- authenticated sender `PeerId`;
- unchanged type string;
- unchanged `timestamp_ns`;
- unchanged payload bytes.

The SDK does not parse, validate, dispatch, or interpret the application fields.

## Catalog compatibility

Resource Catalog v0.2 remains `/auki/resources/0.2.0` with its existing rows and
wire behavior. Resource Catalog v0.3 is the additive
`/auki/resources/0.3.0`: it carries unchanged v0.2 rows plus
`message_channel` rows.

Applications opt into v0.3 through `fetch_resources_catalog_v3` or
`fetch_resources_catalog_v3_with`. There is no silent v0.3-to-v0.2 fallback;
otherwise a peer without channel support would look like a peer with an empty
channel catalog. Such a peer returns an explicit unsupported-v0.3 error.

## Security boundary

For this milestone, any current authenticated cluster member may open and send
to an advertised channel. `NetworkRuntime` membership is the coarse trust
boundary. Unknown peers and peers removed from the allow-list are rejected
before message payloads reach the application receiver.

There is no generic channel-level ACL yet. Applications must not interpret
catalog discovery as finer-grained authorization.

## Clock contract

The channel Resource carries an existing `RegistryRef`. Each message carries
only `timestamp_ns`; there is no per-message clock reference. The reference
round-trips exactly through Resource Catalog v0.3.

Applications use the existing clock declaration and time-transform facilities
to interpret or compare timestamps. The application owns freshness,
scheduling, expiry, and action policy. The SDK does not infer any of them.

## Delivery semantics and exclusions

Send success is a transport ACK: the authenticated receiver runtime accepted
the event into its bounded live queue. It is not application acceptance.
Queueing happens before the ACK is written. If that ACK is lost, send returns
an error while the application receiver may already hold the event. Delivery is
therefore indeterminate on send error, and callers must not automatically retry
because doing so can duplicate an already-enqueued event.

The protocol deliberately provides no:

- history, persistence, or materialization;
- queue across disconnect, retry, or replay;
- delivery after receiver deregistration;
- outcome protocol or application session workflow;
- generic control logic or device-action semantics;
- SDK freshness, scheduling, or action policy.

Failed or disconnected sends are not retained. Rejoining does not replay them.

## Coverage

The Domain integration test uses two loopback peers and an in-process Discovery
stub. It covers explicit v0.3 discovery, unchanged v0.2 behavior, exact clock
round-trip, owner enforcement, the application-level example above, multiple
opaque types over a persistent sender, the one-shot convenience, authenticated
sender identity, exact timestamp and payload delivery, and receiver-drop
failure.

Network tests cover nonmember rejection before payload decoding, disconnect
failure with no replay after reconnection, bounded queues, framing, membership
revocation, and the three-field message wire shape.
