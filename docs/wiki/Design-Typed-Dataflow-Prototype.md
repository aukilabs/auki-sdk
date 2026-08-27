# Experiment: Typed Dataflow Prototype

> **Status: experiment in progress.** The first network-independent prototype
> lives in `experiments/typed-dataflow`. It is not a proposed production API.
> See that crate's `RESULTS.md` for measured findings and open gates.

## Decision this experiment must support

Should the SDK use one network-independent typed dataflow model for local
production, processing, retention, and delivery?

The candidate shape is:

```text
Component OutputPort<T>
  ├─ direct Connection<T> ─→ Component InputPort<T>
  └─ Buffer<T>
       ├─ local Component
       ├─ StreamPump ─→ recipient
       └─ Episode
```

The experiment is successful only if it gives evidence about both sides of
the decision:

1. Can the API make a typed processing graph obvious to an application author
   and to a coding agent?
2. Can its hot paths approach specialized Rust code without copying payload
   bytes, serializing local data, or allocating unnecessarily?
3. Can slow consumers be isolated with bounded and observable behavior?
4. Can a Buffer fan out retained payloads without becoming a mandatory disk or
   chunking boundary?

An attractive example that lacks measurements is not a successful result.

## Hypotheses

### H1 — explicit ports make graphs understandable

An application should be able to read the direction and type of a connection
at the call site:

```rust
connect(
    camera.outputs().frames(),
    detector.inputs().frames(),
    ConnectionOptions::inline_every(),
)?;
```

The type parameter should normally be inferred. Connecting
`OutputPort<CameraFrame>` to `InputPort<AudioFrame>` must fail at compile time.

### H2 — a static inline path can be nearly free

For concrete producer and consumer types, an inline `Every` connection should
be capable of reducing to a borrowed function call:

```text
producer.publish(payload)
  → consumer.accept(&payload)
```

It must not require serialization, heap allocation, reference counting, a
queue, an async task, or a context switch.

### H3 — a dynamic port path has measurable but bounded overhead

Named ports that can be connected at runtime will probably require indirect
dispatch. The experiment must measure this path separately rather than imply
that it receives the same compiler optimization as the static path.

### H4 — owning consumers can share one payload instance

When queued connections or a Buffer must retain a payload beyond the publish
call, the runtime should create at most one shared owned representation. Eight
subscribers must not produce eight copies of the payload bytes.

### H5 — retention can remain off the direct path

A Buffer is an explicit subscriber to an output port. Local Components that do
not need retention may connect directly. A Buffer append may retain a shared
payload reference and notify its subscribers, but the first implementation
must not serialize, chunk, or write to disk.

### H6 — a lightweight Buffer can be the common remote-stream source

Requiring remote Streams to read from a Buffer should add only bounded ring
insertion, sequence assignment, and subscriber notification. It should not add
payload copying, serialization, chunk-seal latency, or disk I/O.

The experiment must compare:

```text
OutputPort → direct live pump
OutputPort → one-entry Buffer → pump from Latest
```

The direct pump is a control implementation, not the presumed public design.
If the lightweight Buffer has negligible cost, its stable identity, sequence
space, replay options, gap reporting, and Episode-promotion path favor making
it the required remote-stream source.

### H7 — batching belongs behind the live Buffer path

Rerun improves storage and transport efficiency by micro-batching small log
calls into immutable chunks, flushing on time or byte thresholds. The useful
hypothesis for this SDK is narrower:

```text
Buffer append
  ├─ notify live subscribers immediately
  └─ asynchronously build immutable retained chunks
```

A StreamPump following `Latest` must not wait for a chunk to seal. Chunked
replay or storage may trade a small amount of latency for throughput, but that
trade must not leak into direct local processing or the live Buffer cursor.

## Non-goals

The first experiment does not include:

- libp2p, Discovery, Domain joining, Managers, membership, or heartbeats;
- the existing network wire protocol;
- authentication or authorization;
- Catalog or Registry schemas;
- disk persistence;
- content hashing;
- Arrow or another specific columnar representation in the first phase;
- a production chunk format;
- production FFI bindings;
- automatic graph scheduling or distributed execution;
- compatibility with current SDK APIs.

These omissions are deliberate. The first question is whether the local data
plane is sound.

## Rerun-informed comparison boundary

Rerun is useful evidence, not the target architecture. Its logging SDK routes
one RecordingStream to [file and gRPC sinks](https://rerun.io/docs/concepts/logging-and-ingestion/sinks),
[micro-batches rows into chunks](https://rerun.io/docs/reference/sdk/micro-batching),
and uses bounded in-memory stores to serve late viewers. Its
[logical recordings](https://rerun.io/docs/concepts/logging-and-ingestion/recordings)
are not deliberately concluded Episodes, and its
["components"](https://rerun.io/docs/concepts/logging-and-ingestion/entity-component)
are ECS data fields rather than executable processors.

This experiment borrows four ideas for direct evaluation:

1. one producer path may fan out to live and retained destinations;
2. retention should be bounded by bytes as well as entries or time;
3. small retained entries may benefit from time-and-size micro-batching;
4. immutable retained storage units may be shared by a Buffer and an Episode.

It does **not** assume that local Component connections should encode Arrow
rows, that all produced data belongs to one logical recording, or that a file
sink defines an Episode.

## Location and isolation

Build the prototype as an experimental Rust crate:

```text
experiments/typed-dataflow/
```

It should have no dependency on `auki-network` or `auki-domain`. Reusing an
existing payload datatype for one integration benchmark is allowed, but the
core abstractions must not depend on today's Log, Manifest, Registry, Catalog,
or Stream types.

## Candidate public vocabulary

The experiment should begin with the following concepts:

```rust
struct OutputPort<T> { /* candidate implementation */ }
struct InputPort<T> { /* candidate implementation */ }
struct Connection<T> { /* owns connection lifetime */ }

enum ConnectionOptions {
    InlineEvery,
    QueuedEvery {
        capacity: usize,
        when_full: EveryFullPolicy,
    },
    Latest,
}

enum EveryFullPolicy {
    Backpressure,
    Disconnect,
}
```

Dropping `Connection<T>` disconnects the ports. Connection lifetime must not be
hidden in an unowned global graph.

The exact Rust representation is part of the experiment. The names above are
the intended application model, not a requirement to force every
implementation through the same struct layout.

### Named direction

Ports must be visibly separated by direction:

```rust
camera.outputs().frames()
detector.inputs().frames()
detector.outputs().detections()
display.inputs().frame()
```

Prefer semantic port names to generic names such as `input`, `output`, or
`data`.

## Delivery semantics

### `InlineEvery`

```text
Produced: A B C D
Called:   A B C D
```

- The publisher invokes the consumer on the publishing thread.
- Every payload is delivered in order.
- The publisher waits for the consumer.
- The consumer receives a borrowed payload.
- The connection has no queue and cannot silently drop.
- Multiple inline consumers run in connection-registration order for the
  prototype.
- A slow or blocking consumer delays the publisher and later inline consumers.

The experiment must document whether a consumer error closes only that
connection or aborts the publish operation. Panics are not recovered across
the connection boundary in the first implementation.

### `QueuedEvery`

```text
publisher → bounded queue → consumer worker
```

- Every accepted payload is delivered once and in order.
- The queue has an explicit positive capacity.
- When full, the connection either applies backpressure or disconnects with an
  explicit overrun error.
- It must never claim `Every` while silently dropping an entry.
- Each queued connection progresses independently from other queued
  connections.

### `Latest`

```text
Produced: A B C D
Pending:          D
```

- The connection retains at most one pending payload.
- A newly published payload replaces an unconsumed pending payload.
- Replacement increments an observable dropped/replaced counter.
- The consumer eventually observes the newest value available while it runs;
  it is not required to observe intermediate values.

## Payload ownership candidates

The prototype must not assume that `T` contains its bytes directly. A camera
payload may be a handle to heap, shared-memory, DMA, or GPU-backed storage.

Test at least these two publication cases:

### Borrowed inline-only publication

If every connection is inline, the publisher should be able to lend the
payload to every consumer without creating shared ownership:

```text
T on producer stack
  ├─ &T → consumer A
  └─ &T → consumer B
```

### Shared owning publication

If any subscriber must outlive the call, create one shared owned payload and
clone only its handle:

```text
shared payload
  ├─ handle → queued consumer A
  ├─ handle → latest consumer B
  └─ handle → Buffer
```

The experiment may use `Arc<T>` initially, but the public model must not imply
that all future payload storage is ordinary heap-owned Rust data.

## Two implementations to compare

### A. Static inline composition

Use concrete generic producer and consumer types with no trait object in the
per-payload call path. This is the candidate that may be monomorphized and
inlined end to end.

The compiled benchmark should be inspected sufficiently to establish whether
the consumer call remains indirect. If it does, the experiment must not call
this a zero-cost path.

### B. Runtime-connectable named ports

Use named `OutputPort<T>` and `InputPort<T>` handles that can be connected after
the Component instances exist. Indirect dispatch is acceptable, but must be
measured independently.

This path represents the ergonomically strongest design for dynamic graphs,
Catalog adapters, and language bindings. The comparison will determine whether
both paths are necessary.

## Buffer scope

The first `Buffer<T>` is a bounded in-memory ring of shared immutable payloads.
It is not a disk Log and does not use chunks.

It must provide:

- append from an `OutputPort<T>` connection;
- a positive entry-capacity limit;
- monotonically increasing sequence numbers;
- one retained payload instance regardless of subscriber count;
- a current retained sequence range;
- subscription from `Latest`, `Current`, or `FromSequence`;
- explicit overrun when a subscriber requests or falls behind evicted data;
- prompt release of the Buffer's ownership when an entry is evicted;
- bounded pending state per subscriber.

The Buffer's ring slot may be reused after eviction, while a payload already
leased to a consumer remains alive until that consumer releases its handle.
Subscriber limits must prevent a stalled consumer from retaining unbounded
history outside the ring.

### Retention-budget extension

After the entry-bounded ring passes, add combined limits:

```rust
struct BufferLimits {
    max_entries: Option<usize>,
    max_bytes: Option<usize>,
    target_duration: Option<Duration>,
}
```

At least one hard bound (`max_entries` or `max_bytes`) is required. A target
duration is a desired window, not permission to exceed the hard memory bound.
The Buffer must expose the actual retained sequence and time range after
eviction.

For the experiment, payload fixtures may report a known retained byte size.
The future SDK will need a truthful accounting interface for external or GPU
storage rather than assuming `size_of::<T>()` measures payload memory.

### Why chunks are staged after the payload ring

A chunk is a possible future physical retention unit, not the definition of a
Buffer. Starting with a payload ring lets the experiment measure the simplest
correct live and retained path.

After the payload-ring baseline is measured, introduce a small asynchronous
chunk-builder comparison only for retained replay and Episode promotion. It
should be justified against one or more of:

- large Buffer memory overhead;
- expensive Episode promotion;
- replay/indexing throughput;
- repeated network encoding;
- disk persistence.

The Buffer remains the semantic data product in both implementations:

```text
Buffer
  identity + provenance + retention policy + available range + cursors
    └─ storage implementation
         ├─ shared payload ring
         └─ open builder + immutable sealed chunks
```

A chunk has no Catalog identity or retention policy in this experiment. When
the Buffer evicts a sealed chunk it releases its ownership. If an Episode also
references that chunk, the chunk remains alive until the Episode releases it.

## Transport-neutral StreamPump

After the local port and Buffer measurements pass their correctness checks,
add an in-memory StreamPump experiment:

```text
Peer-like A                                      Peer-like B

Camera OutputPort
  → Camera Buffer
      → StreamPump → bounded in-memory sink → Remote Camera Buffer
                                               → Detector input
```

This is not a network test. The sink deliberately models only the transport
properties the data plane must handle:

- asynchronous acceptance;
- bounded capacity;
- cancellation;
- receiver failure;
- optional delay;
- an observable delivered sequence;
- a gap when the receiver cannot keep up.

Use one logical pump per recipient. Multiple pumps may reference the same
Buffer payload, but each owns its delivery progress, cancellation, and
backpressure behavior.

### Direct-pump control

Implement a control path that subscribes a pump directly to an OutputPort with
live-from-now semantics. Compare it with a pump following a one-entry Buffer
from `Latest`.

The comparison must answer:

- Does the Buffer add an allocation or payload-byte copy?
- Does it materially change publish-to-sink latency?
- Can both paths isolate a stalled recipient equally well?
- Does direct pumping duplicate sequence, gap, or fan-out machinery already
  required by the Buffer?

The result may preserve the direct path, but it should do so because of measured
benefit rather than because another logging system models network delivery as a
sibling sink.

## Episode promotion extension

After the Buffer experiment is correct, add the smallest possible Episode
model:

```text
Buffer with sequence range [100, 200]
  → promote [150, 200]
  → Episode initially references the same retained payloads
  → Episode continues accepting new payloads
  → Episode concludes at sequence 260
```

For this first version, promotion may retain shared per-payload handles. It
must not copy the payload bytes. Whether a later implementation should share
immutable chunks is a separate benchmark-driven decision.

For the chunk-builder extension, repeat promotion after several chunks have
sealed. Confirm that the Buffer and Episode share those chunks, while any open
partial chunk is handled explicitly. Record whether sealing or copying that
partial chunk creates a latency or memory spike.

## Demonstration graph

The executable example should use this graph:

```text
FakeCamera output.frames
  ├─ InlineEvery  → MeanBrightness input.frames
  │                   → output.level
  │                       → Level Buffer
  ├─ Latest       → SlowPreview input.frames
  └─ QueuedEvery  → Camera Buffer
                       ├─ StreamPump → Remote Camera Buffer
                       │                → RemoteDetector input.frames
                       └─ promoted Camera Episode
```

The program should print or expose only evidence useful to the experiment:
delivered sequences, replacements, overruns, retained ranges, and pointer or
storage identity. It should not introduce a Catalog UI or network simulation.

## Workload matrix

Use three payload profiles:

| Profile | Representative shape | Purpose |
|---|---:|---|
| Small | 16–32 byte scalar/IMU observation | expose dispatch, allocation, and atomic overhead |
| Medium | 4–16 KiB audio block | exercise ordinary queued delivery |
| Large | approximately 6 MiB RGB frame handle | detect payload copying and memory retention |

Large payload bytes should be allocated from a small reusable fixture pool
outside the timed benchmark region. The benchmark must still use distinct
payload identities and sequences; repeatedly publishing one constant pointer
is insufficient to test retention behavior.

Run each relevant case with:

- one consumer;
- eight consumers;
- one and three processing stages;
- all consumers keeping up;
- one consumer deliberately stalled;
- Buffer capacities of `1`, `60`, and a larger stress capacity appropriate to
  the host running the benchmark.

For the retained small-payload cases, compare:

- one stored object per payload;
- chunk flushing by entry count;
- chunk flushing by elapsed time;
- chunk flushing by encoded byte threshold;
- whichever of time or byte threshold fires first.

The experiment should not assume Rerun's particular default thresholds are
appropriate for robotic data.

## Baselines

Compare against:

1. A handwritten direct concrete function call.
2. A purpose-built bounded queue for the same producer and consumer.
3. The current SDK's `CameraFrameHub` for shared-reference latest delivery.
4. The current stored-Log detector input path as a separate end-to-end
   reference; label its disk I/O and decoding costs rather than conflating them
   with dispatch overhead.
5. A direct OutputPort-to-pump control against a one-entry Buffer-to-pump path.
6. Per-payload retained storage against the experimental immutable chunk
   builder for small, medium, and large payload profiles.

## Measurements

Collect:

- messages per second;
- publish-call duration;
- end-to-end p50 and p99 delivery latency for queued paths;
- allocations per published payload;
- bytes copied per published payload where measurable;
- CPU time;
- peak retained memory;
- replacement/drop/overrun counts;
- shutdown and cancellation completion time.
- chunk-fill ratio, chunk count, and time-to-first-live-delivery for chunked
  retained cases.

Report results separately for each payload profile and connection mode. A
single blended throughput number is not useful.

## Correctness tests

The prototype must include automated tests proving:

1. Incompatible port types do not compile.
2. `InlineEvery` delivers every sequence once and in order.
3. `QueuedEvery` delivers every accepted sequence once and in order.
4. A full `QueuedEvery` connection never silently drops.
5. `Latest` eventually delivers the newest sequence and reports replacements.
6. Eight consumers observe the same large payload storage identity.
7. A slow consumer does not block unrelated queued or latest consumers.
8. A Buffer never exposes an evicted sequence as retained.
9. A Buffer reports a gap when a cursor falls behind its retained range.
10. A stalled subscriber cannot cause unbounded memory growth.
11. Dropping a Connection stops future delivery and releases retained handles.
12. An Episode promotion shares payload storage with its source Buffer.
13. A StreamPump cancellation affects only its recipient.
14. A remote-style receiving Buffer preserves source sequence and gap evidence.
15. A one-entry Buffer-to-pump path does not copy payload bytes.
16. Live pump delivery does not wait for retained chunk sealing.
17. Buffer eviction releases its chunk ownership while an Episode reference
    keeps the shared chunk alive.
18. Combined entry/byte limits never exceed their hard configured bounds,
    allowing only explicitly documented accounting slack.

## Performance decision gates

Before running the benchmarks, record the host, compiler, optimization profile,
and CPU-affinity settings. Compare like with like.

The initial gates are:

- Static `InlineEvery` has no heap allocation per payload and is within 5% of
  the handwritten direct-call baseline for the small payload benchmark.
- Fan-out does not copy large payload bytes solely because another consumer is
  attached.
- All queue and Buffer configurations remain within calculable bounded memory
  when a consumer stalls indefinitely.
- `Latest` and overrun counters exactly account for payloads the consumer did
  not receive in deterministic tests.
- The publishing hot path performs no serialization or disk I/O.
- A one-entry Buffer-to-pump path is compared directly with a live-from-now
  OutputPort pump. If the Buffer path is materially slower, the universal
  remote-stream-source rule must be reconsidered.
- Retained chunk batching must improve a measured storage, replay, encoding, or
  memory metric enough to justify its additional lifecycle complexity.
- Live subscribers receive entries before or independently of chunk sealing.
- The runtime-connectable implementation's cost is reported rather than hidden.
  If its overhead is unacceptable for small payloads, the result should favor a
  two-path design rather than weakening the measurements.

The 5% figure is an experiment threshold, not a universal SDK performance
requirement. If measurement noise is larger, fix the benchmark before drawing
an architectural conclusion.

## Agent-friendliness check

Give the public API documentation—but not the prototype implementation—to a
coding agent and ask it to add:

1. a typed `FakeMicrophone` output;
2. a `Volume` Component with an explicitly named audio input and level output;
3. a latest-only level display;
4. a 60-entry audio Buffer;
5. one deliberately invalid audio-to-camera connection test.

Record:

- whether the first implementation compiles;
- incorrect assumptions made about direction, retention, or ownership;
- whether the agent invents string type names or bypasses typed ports;
- how much additional instruction was necessary.

This is qualitative evidence, but it directly tests the stated goal that the
SDK should guide agents toward compliant construction.

## Deliverables

The experiment ends with:

1. the isolated prototype crate;
2. the executable demonstration graph;
3. correctness and compile-fail tests;
4. reproducible benchmarks and raw results;
5. a short report comparing static, dynamic, and current SDK paths;
6. a comparison of direct and Buffer-backed StreamPump sources;
7. a comparison of per-payload and chunk-backed retained storage;
8. a decision: reject the model, adopt one implementation, or retain distinct
   static and dynamic connection paths;
9. a list of unresolved questions before Catalog, Registry, production storage,
   or real networking work begins.

## Stop conditions

Stop and reassess rather than expanding the prototype if:

- typed named ports require serialization on local connections;
- payload fan-out requires byte copies per consumer;
- memory cannot be bounded under a stalled consumer;
- static inline composition cannot approach a direct call;
- delivery semantics cannot be stated without application-specific exceptions;
- the simple demonstration requires hidden global state or a general graph
  scheduler;
- a chunk store becomes necessary merely to implement basic live fan-out.

The purpose of the prototype is to learn whether the model deserves to become
the SDK's center. It is not to accumulate enough experimental code that the
model becomes difficult to reject.
