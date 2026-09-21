# Network typed Components

`auki-components` defines the network-independent execution model:
Components declare typed Product inputs, Observables, and Operables; configured
Observables produce immutable Products; configured inputs consume those
Products; and a read-only Catalog projects the live topology.

`auki-component-protocol` is an application-protocol layer on top of
`AukiPeer`. It is deliberately separate from the manager-era
`auki-protocols` crate and uses four exact protocols:

| Protocol | Purpose |
| --- | --- |
| `/aukilabs/components/catalog/1.0.0` | Discover the exported Component/Product surface by revision |
| `/aukilabs/components/observations/2.0.0` | Read observations and terminal source notices from one exact Buffer Product |
| `/aukilabs/components/observation-stream/1.0.0` | Subscribe once to observations from one exact Buffer Product |
| `/aukilabs/components/operations/1.0.0` | Invoke one typed Operable on one exact Component |

## Layering

```text
application policy and UI
          |
          +-- ComponentRuntime
          |     Components -> Observables -> Products
          |     Products   -> typed inputs -> Components
          |     caller     -> Operables     -> actuators
          |
          +-- ComponentProtocolEndpoint / Client
                       |
                    AukiPeer
          authenticated Domain + Peer ID
          routes + discovery + protocol streams
```

The transport does not become part of local component execution. A local
producer and a remote producer both reach a consumer as a
`RetainedProduct<T>` bound through `configured_buffer_input`. The remote mirror
retains the source Product manifest, producer Output manifest, sequence, and
timestamp. When source retention has already evicted requested observations,
the protocol returns an explicit `SourceGap` before continuing at the first
available sequence.

The mirror does not secretly create a polling task. Its host calls `sync_once`
and therefore owns cadence, retry, cancellation, and backoff. The Component
runtime continues to own type/schema compatibility checks and input delivery.

Both experimental crates live under `labs/`. Stable `core/` crates do not
depend on them. Applications can fetch finite ranges, poll, or subscribe to a
continuing sequence. All three paths currently read retained Buffer Products;
this is not a direct, unretained Component-output media transport.

## Polling: continuity, retries, and termination

`sync_once` requests the next bounded batch of source sequences. It advances
the mirror's cursor only after each observation is retained locally. If an
append fails, the accepted prefix remains available and the next attempt starts
at the first unaccepted sequence. Retention eviction on the source is still
possible during a retry and is reported as a `SourceGap`, not hidden.

`sync_latest_once` deliberately jumps to the newest available observation,
reporting any skipped sequence interval. Repeating it with no new data is a
successful no-op (`accepted == 0`), not a duplicate-sequence error.

Reconfiguration or producer failure closes the source Buffer with an
`ObservationEnd`. Its reason is runtime state shared by retained Product handles,
not a mutation of the immutable Product manifest. For a sequence request the
endpoint includes the notice only when the batch reaches the retained tail.
The mirror appends that batch before closing its local readers; a failed append
does not prematurely consume the end notice. Readers can drain retained values
and then see closure. `RemoteProductSync::end`, `mirror.end_notice()`, and
`RetainedProduct::end_notice()` expose the reason and exact source reference.

Finite queries can still read ended Products while they remain exported.
Reconfiguration does not transfer a reader to a replacement Product. The host
must discover and bind a new Product explicitly. After receiving an end notice,
further sync calls return the same terminal state without network requests.

Notices are delivered on the next successful poll. This does not add unsolicited
push, automatic reconnection, or a new remote-cancellation protocol. Calling
`mirror.close()` closes local readers and releases its transport, without
claiming that the remote producer ended.

## Continuing observation: one request, then pushed events

`subscribe_product_exact` sends one subscription request. The provider then
awaits Buffer change notifications and pushes observations in source sequence
order. There is no polling interval, extra history queue, or thread per network
subscriber. The host drives `subscription.next().await` in its own async task;
`next()` reads pushed events and does not send another request.

```rust,ignore
let mut subscription = client.subscribe_product_exact::<AudioChunk>(
    expected_peer, advertised_route, audio_product,
    ObservationStart::LatestExisting,
    BufferLimits::entries(64),
    |chunk| chunk.retained_bytes(),
).await?;

let input = consumer.configured_buffer_input(
    "audio", subscription.product(),
    CursorStart::FromSequence(subscription.next_sequence()), &audio_input,
)?;

while let Some(event) = subscription.next().await? {
    // Data is already retained. The ordinary Component input reads that Buffer.
    // The host also handles explicit Gap / Closed events.
    handle_event(event);
}
subscription.close().await?;
```

`AudioChunk`, its size function, the input, and the event handler are
application-defined. Arrange cleanup on errors and user cancellation too:
`close().await` closes an idle stream; dropping the handle always releases
transport ownership and closes local readers.

Initial selection is explicit:

- `FromSequence { sequence }`: replay from that source sequence, then follow.
  Evicted or missing observations produce a `Gap`.
- `LatestExisting`: include the newest retained observation, then follow.
- `NewOnly`: start after the provider's high-water mark when it accepts the
  request. This reads an already-running producer; it does not start capture.

Use the subscription's initial `next_sequence()` when attaching a local reader
to an empty imported Buffer; its first source sequence need not be zero.

`RemoteObservationEvent<T>` distinguishes data, `Gap(SourceGap)`, and
`Closed(Option<ObservationEnd>)`. Reconfiguration/failure supplies its terminal
notice after retained data drains. Capture closure without a producer notice is
`Closed(None)`. Local readers close; retained data and manifests remain readable.
No subscriber follows a replacement automatically.

Cancelling a pending `next()` wait preserves partial framing for the next call.
A local append failure keeps the same observation pending without advancing
the cursor: adjust retention limits and retry. Other receive/contract/transport
errors terminate the relationship. Resume requires a new explicit subscription,
optionally from `next_sequence()`; the old local Buffer is not silently replaced
or merged. There is no automatic reconnect or route migration.

Slow consumers neither stop capture nor pin its entire history. The provider
holds one current encoded observation per subscriber while a network write is
pending, in addition to the shared bounded source Buffer and transport buffers.
If capture outruns delivery, subsequent cursor reads report gaps. This is not a
lossless `Every` guarantee, adaptive bitrate, or a real-time latency guarantee.
JSON encoding is unchanged; media codecs and binary payload transport are later
work.

Dropping/closing a subscription releases its handler even while the source is
idle. `unexport_product` interrupts active subscribers too. A partly written
frame cannot safely carry a new error message, so withdrawal closes transport;
clients report an error rather than a fictitious producer end. Endpoint shutdown
cancels admitted handlers through the existing SDK registration lifecycle.
`active_subscriptions()` exposes the handler count for diagnostics. These
actions do not stop the source or another subscriber.

### Network lifetime is not producer lifetime

Keep these events separate in the host application:

- **Credential replacement:** a valid `ExternalAuthorityControl::replace` for
  the same Peer and Domain is not Component reconfiguration. It does not change
  Product manifests or require ending a healthy subscription. Rejected authority
  replacements do not install a different Peer or Domain.
- **Detected connection failure or peer shutdown:** `next()` returns an error,
  the imported Buffer closes, and subsequent calls return `None`. Previously
  retained data remains readable. No remote `ObservationEnd` is invented:
  the producer may still be running. The host must preserve the error if it
  needs to explain why local readers closed.
- **Cancellation:** cancelling one pending `next()` only cancels that wait;
  `close()` or dropping the subscription ends the relationship. Other
  subscribers and the producer remain independent.
- **Reconfiguration:** the source sends its actual terminal notice. Observing
  the replacement requires selecting its new Product reference, creating a new
  subscription, and explicitly binding any consuming Component to that Product.

After a connection failure, the host may select a working route and create a
new subscription with `FromSequence { sequence: old.next_sequence() }`. This
resumes from the next observation accepted into the old local Buffer, **not**
an acknowledgement that every downstream Component processed it. If the source
has evicted that position, the new subscription reports a `Gap`. The new import
has separate local retention; it does not reopen or append to the closed Buffer.
Reusing a sequence applies only to the same exact Product, not a replacement.

There is no subscription-level heartbeat or idle deadline. A silent partition
that produces no transport error can leave an idle `next()` pending; the
60-second started-frame deadline does not cover that wait. Applications needing
a freshness deadline must choose one explicitly and close the subscription
when it expires. A quiet sensor and a lost connection cannot be distinguished
from observation silence alone.

## Serving

Mounting an endpoint registers all four protocols. Serving remains explicit:

```rust,ignore
let endpoint = ComponentProtocolEndpoint::mount(peer.protocols(), runtime)?;
endpoint.export_product(&camera_history.product())?;
endpoint.export_operable(&set_frame_rate)?;
```

The network Catalog contains only explicitly exported Products and Operables,
plus the exact manifests of their owning Components. Unexported local Products
and unrelated Components are not projected. Export or unexport changes advance
the network Catalog revision independently of local runtime changes.

Close the Component endpoint before shutting down its `AukiPeer`:

```rust,ignore
endpoint.close().await?;
peer.shutdown().await?;
```

## Calling

DDS discovery may locate peers advertising the exact Catalog protocol, but its
routes are hints. The application chooses the expected Peer ID and route, then
uses `catalog_exact`, `observations_exact`, `mirror_product_exact`,
`subscribe_product_exact`, or
`invoke_exact`. Mutual authentication verifies which Peer and Domain answered.

An operation request may name a caller Component, but cannot claim a caller
Peer. The endpoint creates `InvocationContext.caller_peer_id` exclusively from
the authenticated stream. The Operable's normal authorization closure makes
the application-specific decision using that Peer ID and caller Component ID.

`Exposure::Cluster` makes an interface eligible for export; it is not a grant
to every Domain peer. Applications still decide which Products to export and
which callers each Operable authorizes.

## Compatibility and bounds

The observations protocol changed from 1.0.0 to 2.0.0 to carry terminal notices.
Provider and consumer must upgrade together; the endpoint registers only 2.0.0
and the client never silently falls back to 1.0.0. Mixed versions fail protocol
negotiation for observations. Catalog and Operations remain at 1.0.0. This is an
experimental application-protocol change, not a change to `AukiPeer` transport,
credential validation, discovery, or any backend service contract.

Continuing observation is an additive `observation-stream/1.0.0` protocol.
Finite observations v2, Catalog, and Operations are unchanged. Both peers must
support the new ID to subscribe. Older endpoints can still serve finite
requests; the subscription client never silently falls back to polling.

- JSON control frames: 1 MiB maximum.
- Typed payload frames: 32 MiB maximum.
- Observation batches: 4,096 entries maximum.
- Remote operation deadlines: 60 seconds maximum.
- Concurrent inbound handlers: bounded by the Auki protocol runtime.
- Continuing observation: at most 32 handlers, each for one exact Product.
- Subscription handshake and close: 10-second deadlines. Payload writes and
  reception after a frame starts: 60 seconds. An idle source may remain quiet
  indefinitely; the host may cancel its wait or the whole subscription.

Changing encoding or framing is a protocol-version change. Adding a new
Component datatype or Product schema is normally an application contract
change carried inside the existing bounded payload envelope.

## Proof

Run the self-contained authenticated two-peer test:

```sh
cargo test -p auki-component-protocol --test two_peer
```

It proves exact Peer rejection, filtered and revisioned Catalog snapshots,
remote Product import into an ordinary Component input, source continuity,
authorized remote invocation, and rejection of an unauthorized caller.
Regression cases cover repeat-latest reads, source eviction gaps, local append
failures (including a partially accepted final batch), paged terminal draining,
reconfiguration without migration, and empty ended Products.
Subscription tests cover push into an ordinary Component input, independent
subscribers, cancellation mid-frame, retention retry, termination, withdrawal,
and endpoint shutdown. Cursor tests verify wake-up and cancellation cleanup,
shared envelopes, and slow-reader gaps.

The native two-peer suite also exercises authority replacement on both peers,
rejection of wrong-Peer/wrong-Domain updates, either peer shutting down, and a
transparent loopback TCP proxy that is cut after authentication. It checks
retained-tail readability after transport loss, explicit resubscription with
an eviction gap, and Catalog selection of a replacement after reconfiguration.
These tests use fixture-signed credentials and no shared services. They do not
prove browser behavior, silent-partition detection, or the combined live
DDS/DMS relay-renewal path. Core relay tests provide separate lower-layer
coverage; neither set substitutes for an authorized deployment test.
