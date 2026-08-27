# Observable and Operable Component data-plane experiment

This crate tests a network-independent data plane for SDK Components and data
products. It deliberately does not use the current Log, Registry, Catalog,
Domain, Manager, or networking implementations.

The second slice adds a sharper public model while retaining the first
prototype as a performance and ownership baseline:

- a **Peer** is an authenticated-runtime-shaped test fixture;
- a **Component** is stable composable behavior hosted by a Peer;
- an **Observable** can show typed observations to another Component;
- an **Operable** lets another Component intentionally configure or execute
  behavior;
- **exposure** determines whether an interface is discoverable through the
  experimental Catalog;
- a configured **Component Output** has an identity and immutable Manifest
  separate from its Component;
- a **Product Manifest** references the exact Output that produced it.

Phase 1 adds explicit observation selection and lifecycle:

- a fresh Component Observable advertises `FollowNew` only;
- a Buffer remains a Product and separately offers `LatestExisting` and
  clock-qualified `TimeRange` access;
- `ObservationHandle<T>` reports active, reconfigured, and cancelled state;
- `EverySelected` and `CoalesceLatest` name delivery behavior rather than
  selection;
- a serialized in-memory fixture counts encoded messages and bytes instead of
  pretending an `Arc` crossed a network without copying.

Phase 2A stresses observation delivery and failure without touching production
networking:

- `SharedScheduler` runs many relationships on a fixed worker pool;
- each relationship still owns an explicit bounded `EverySelected` queue or
  one-slot `CoalesceLatest` queue;
- returned observer errors and observer panics close only the affected
  relationship and become inspectable through its handle;
- a producer can report a terminal Output failure;
- Buffer eviction and cancellation release ownership without invalidating
  payload leases held elsewhere.

The Camera vertical slice demonstrates why Component and Output identity are
separate:

```text
Camera Component @ stable Component Manifest hash
  Operable: set_resolution
  output slot: frames
    frames-1 @ Output Manifest hash 1  --Reconfigured-->
    frames-2 @ Output Manifest hash 2
```

A pinned observation of `frames-1` ends at the explicit transition. An
opt-in follow-current observation crosses to `frames-2` while reporting both
identities. Buffer Products roll at the same boundary so one Product Manifest
never claims observations produced under two Output contracts.

```text
Component OutputPort<T>
  ├─ Connection<T> ─→ Component InputPort<T>
  └─ Buffer<T>
       ├─ BufferReader<T> ─→ local Component InputPort<T>
       ├─ StreamPump<T> ─→ recipient Buffer<T>
       └─ promote shared entries ─→ Episode<T>

Buffer<T> ─→ asynchronous ChunkBuilder<T>
```

The Buffer is a semantic retained data product. A Chunk is only an
experimental physical storage unit. Live delivery never waits for a chunk to
seal.

## Component-facing API

Components expose direction and meaning at the call site:

```rust,ignore
let connection = connect(
    camera.outputs().frames(),
    detector.inputs().frames(),
    ConnectionOptions::InlineEvery,
)?;
```

The payload type is inferred. An `OutputPort<CameraFrame>` cannot connect to an
`InputPort<AudioFrame>`. The connection owns its lifetime; dropping it stops
future delivery.

Delivery is explicit:

- `InlineEvery` borrows each payload and calls the consumer synchronously.
- `QueuedEvery` owns a bounded queue and either backpressures or disconnects
  when full. It never silently drops while claiming to deliver every value.
- `Latest` owns one pending slot, replaces stale pending values, and counts
  every replacement.

`connect_shared` and `Observable::follow_new_shared` preserve those delivery
semantics while submitting drain work to a fixed pool. The pool deliberately
processes one observation per scheduled turn to make fairness visible. That
choice is tested, not presented as the final scheduler policy.

`StaticConnection<T, F>` is a separate concrete path used to test whether a
fully static local graph can compile down near a direct function call.

## Buffer behavior

A Buffer retains shared immutable envelopes. It is bounded by entries, bytes,
or both, and may also target a time window. Byte limits use an explicit
payload-size accounting function; the exposed range labels this value
`retained_payload_bytes` rather than claiming it measures allocator or external
storage overhead. Cursors start at `Latest`,
`Current`, or `FromSequence(n)`. A cursor holds only its next sequence and its
currently leased envelope; it cannot pin an unbounded private backlog.

Eviction and transport loss remain visible because source sequence numbers are
preserved. A reader that falls behind receives an explicit gap with both the
requested sequence and the first sequence still available.

## Run it

```sh
cargo run -p auki-typed-dataflow-experiment --bin typed-dataflow-demo
cargo run -p auki-typed-dataflow-experiment --bin observable-operable-demo
cargo run --release -p auki-typed-dataflow-experiment \
  --bin observation-request-bench -- --iterations 100000
cargo run --release -p auki-typed-dataflow-experiment \
  --bin dataflow-scheduler-stress
cargo test -p auki-typed-dataflow-experiment --all-targets
cargo test -p auki-typed-dataflow-experiment --doc
cargo run --release -p auki-typed-dataflow-experiment \
  --features current-sdk-baseline \
  --bin typed-dataflow-bench -- --iterations 20000000
```

The optional benchmark feature imports the current `CameraFrameHub` only as a
comparison baseline. The core experiment has no dependency on current SDK
data-plane or network types.

See [`RESULTS.md`](RESULTS.md) for the first typed-port and Buffer measurements
and [`RESULTS-OBSERVABLE-OPERABLE.md`](RESULTS-OBSERVABLE-OPERABLE.md) for the
Component/Output identity vertical slice. Phase 1 observation-selection and
lifecycle results are in
[`RESULTS-OBSERVATION-REQUESTS.md`](RESULTS-OBSERVATION-REQUESTS.md). Phase 2A
failure and scheduler results are in
[`RESULTS-DATAFLOW-STRESS.md`](RESULTS-DATAFLOW-STRESS.md).

This code is intentionally disposable. The design should be rejected or
changed if the evidence does not support it.
