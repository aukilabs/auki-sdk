# Auki Component Protocol

Standalone, portable application protocols that expose the network-independent
`auki-components` model through mutually authenticated `AukiPeer` streams.

The protocol family is intentionally separate from the manager-era
`auki-protocols` crate:

- `/aukilabs/components/catalog/1.0.0`
- `/aukilabs/components/observations/2.0.0`
- `/aukilabs/components/operations/1.0.0`

The authenticated stream supplies the caller peer identity. Wire messages may
identify a caller Component, but they cannot assert or override the caller peer.

## What is on the wire

The family keeps three concerns separate:

- **Catalog** returns a revisioned projection containing only Products and
  Operables explicitly exported by this endpoint, plus their owning Components.
- **Observations** reads one immutable Product reference by latest value, time
  range, or source sequence. Source sequence/timestamp and explicit retention
  gaps survive transport, along with the source's terminal notice.
- **Operations** invokes one exact Component reference and typed Operable. The
  provider's authorizer receives the caller Peer ID from the authenticated
  stream and the caller Component ID from the request.

Payloads are typed by the Component/Product manifests and encoded
as bounded JSON frames. Control frames are limited to 1 MiB, payload frames to
32 MiB, observation batches to 4,096 entries, and operation deadlines to 60
seconds.

## Endpoint and client

`auki-components` remains completely network-independent. An application owns
the explicit bridge:

```rust,ignore
let endpoint = ComponentProtocolEndpoint::mount(peer.protocols(), runtime)?;
endpoint.export_product(&capture.product())?;
endpoint.export_operable(&set_frame_rate)?;

let client = ComponentProtocolClient::new(other_peer.protocols());
let catalog = client
    .catalog_exact(expected_peer, advertised_route.clone(), None)
    .await?;
```

For ongoing dataflow, `mirror_product_exact` creates a local
`RetainedProduct<T>` that preserves the remote Product and producer manifests.
The consuming Component binds that Product with ordinary
`configured_buffer_input`; it does not need a network-specific input API. The
host calls `sync_once` at its chosen cadence and owns retry/backoff policy.

This is retained-Product polling, not a continuing push subscription or a
fresh-frame streaming API. `sync_latest_once` deliberately skips to the newest
retained observation and reports skipped source sequences. Reading the same
latest observation again accepts zero new observations. A failed local append
leaves that sequence pending for retry; successfully retained prefixes are not
appended twice.

Reconfiguration or source failure travels in the final batch as an
`ObservationEnd`. A sequential mirror drains the retained tail before closing
its local readers. Inspect `RemoteProductSync::end` or `mirror.end_notice()` for
the reason. Reconfiguration never migrates a mirror to the replacement Product;
that requires explicit discovery and a new mirror. Ended Products remain
fetchable while exported, and their manifests do not change.

The observations protocol is now **2.0.0**: both peers must upgrade together.
There is no fallback to 1.0.0, whose response cannot convey terminal notices.
Catalog and Operations remain 1.0.0. No core networking or authentication
contract changes are required. A source end arrives on the next successful
poll, not asynchronously. Local mirror cancellation closes its readers but is
not a new terminal source reason on the wire.

DDS discovery only supplies short-lived candidates. Use an expected Peer ID
and an exact advertised route for the protocol operation; the authenticated
dial is the authority check.

## Verification

```sh
cargo test -p auki-component-protocol
cargo clippy -p auki-component-protocol --all-targets --no-deps -- -D warnings
cargo check -p auki-component-protocol --target wasm32-unknown-unknown
```

`tests/two_peer.rs` starts two authenticated peers on loopback and proves
Catalog revisioning/filtering, exact Peer enforcement, remote Product mirroring
into a normal typed input, authorized remote operation, and rejection of an
unauthorized caller Component.
It also covers repeat-latest no-ops, source eviction gaps, append retries,
terminal notices across multiple batches, empty ended Products, and readers
draining before closure.
