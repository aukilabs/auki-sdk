# Typed dataflow experiment

This crate tests a network-independent data plane for SDK Components and data
products. It deliberately does not use the current Log, Registry, Catalog,
Domain, Manager, or networking implementations.

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
cargo test -p auki-typed-dataflow-experiment --all-targets
cargo test -p auki-typed-dataflow-experiment --doc
cargo run --release -p auki-typed-dataflow-experiment \
  --features current-sdk-baseline \
  --bin typed-dataflow-bench -- --iterations 20000000
```

The optional benchmark feature imports the current `CameraFrameHub` only as a
comparison baseline. The core experiment has no dependency on current SDK
data-plane or network types.

This code is intentionally disposable. The design should be rejected or
changed if the evidence does not support it.
