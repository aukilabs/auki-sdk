# Network typed Components

`auki-components` defines the network-independent execution model:
Components declare typed Product inputs, Observables, and Operables; configured
Observables produce immutable Products; configured inputs consume those
Products; and a read-only Catalog projects the live topology.

`auki-component-protocol` is an application-protocol layer on top of
`AukiPeer`. It is deliberately separate from the manager-era
`auki-protocols` crate and uses three exact protocols:

| Protocol | Purpose |
| --- | --- |
| `/aukilabs/components/catalog/1.0.0` | Discover the exported Component/Product surface by revision |
| `/aukilabs/components/observations/2.0.0` | Read observations and terminal source notices from one exact Buffer Product |
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
depend on them. This API polls retained Buffer Products; it does not yet offer
a continuing push subscription or fresh-frame streaming.

## Continuity, retries, and termination

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

## Serving

Mounting an endpoint registers all three protocols. Serving remains explicit:

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
uses `catalog_exact`, `observations_exact`, `mirror_product_exact`, or
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

- JSON control frames: 1 MiB maximum.
- Typed payload frames: 32 MiB maximum.
- Observation batches: 4,096 entries maximum.
- Remote operation deadlines: 60 seconds maximum.
- Concurrent inbound handlers: bounded by the Auki protocol runtime.

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
