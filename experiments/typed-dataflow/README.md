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
- the current experiment gives a configured **Component Output** an identity
  and immutable Manifest separate from its Component, but that split is now
  explicitly provisional;
- a **Product Manifest** references the exact Output that produced it.

Phase 1 adds explicit observation selection and lifecycle:

- a fresh Component Observable advertises `FollowNew` only;
- a Buffer remains a Product and separately offers `LatestExisting` and
  clock-qualified `TimeRange` access;
- `ObservationHandle<T>` reports active, ended, failed, and cancelled state;
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

The complete network-independent pass adds the public construction and
retention APIs that the first clean-room attempt found missing:

- `PeerRuntime::component` constructs a live `Component` without mutating the
  Catalog;
- `Component::configured_observable` and `Component::operable` bind typed live
  behavior to declared contracts;
- `Component::expose` projects into the Catalog only after every declared
  interface is live;
- direct Catalog mutation is crate-private;
- `ContractType` prevents a Rust payload or instruction type from contradicting
  its advertised datatype;
- camera, audio, Gauge, and structured payload contracts have distinct typed
  fields instead of one optional-property bag;
- `PeerRuntime::capture_buffer` and `capture_episode` construct live Product
  behavior and Catalog entries together;
- Operables support bounded outstanding work, serial acceptance ordering,
  inspectable asynchronous completion, cancellation, deadlines, and failure;
- Buffer duration retention declares source-time or arrival-time policy;
- external storage is exercised with explicit leases, accounting, and
  serialized readback.

The Camera vertical slice currently represents a configuration change with a
replacement Output:

```text
Camera Component @ stable Component Manifest hash
  Operable: set_resolution
  output slot: frames
    frames-1 @ Output Manifest hash 1  --Reconfigured-->
    frames-2 @ Output Manifest hash 2
```

Every observation of `frames-1` ends with an explicit reconfiguration notice.
The notice may identify `frames-2`, but neither the consumer nor its Buffer is
migrated. The application must deliberately create a new subscription and, if
desired, attach a new Buffer to `frames-2`. One Product Manifest therefore
never claims observations produced under two configured contracts.

This simpler lifecycle removes the main reason for using a stable named slot
as a subscription anchor. Separate Output identity and named slots remain in
the fixture only so the next design review can test whether Products and
multi-output Components still justify them.

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

Source timestamps are mandatory. A Buffer chooses `StrictlyIncreasing`,
`NonDecreasing`, or `Unordered` source-time handling and independently chooses
source or arrival time for duration eviction. Unordered source time cannot be
used for source-time duration eviction.

## Run it

```sh
cargo run -p auki-typed-dataflow-experiment --bin typed-dataflow-demo
cargo run -p auki-typed-dataflow-experiment --bin observable-operable-demo
cargo run -p auki-typed-dataflow-volume-monitor
cargo run --release -p auki-typed-dataflow-experiment \
  --bin observation-request-bench -- --iterations 100000
cargo run --release -p auki-typed-dataflow-experiment \
  --bin dataflow-scheduler-stress
cargo run --release -p auki-typed-dataflow-experiment \
  --bin operable-bench -- --iterations 50000
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

The complete network-independent pass, second clean-room implementation,
timestamp/external-storage evidence, chunk recommendation, and latest benchmark
controls are recorded in
[`RESULTS-COMPLETE-PROTOTYPE.md`](RESULTS-COMPLETE-PROTOTYPE.md).

The remaining blind agent-friendliness validation is specified by
[`CLEAN-ROOM-VOLUME-MONITOR-TASK.md`](CLEAN-ROOM-VOLUME-MONITOR-TASK.md) and
[`CLEAN-ROOM-EVALUATION-RUBRIC.md`](CLEAN-ROOM-EVALUATION-RUBRIC.md). Its
initial public-API feasibility gate failed before a generic live Component
could be constructed. That failure and the now-completed reviewed second pass
are recorded in [`CLEAN-ROOM-FIRST-ATTEMPT.md`](CLEAN-ROOM-FIRST-ATTEMPT.md).

This code is intentionally disposable. The design should be rejected or
changed if the evidence does not support it.
