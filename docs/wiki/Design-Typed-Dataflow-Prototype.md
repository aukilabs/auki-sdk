# Experiment: Observable and Operable Component Data Plane

> **Status: phased experiment with Phase 1 implemented.** The first
> typed-dataflow prototype, Component/Output identity vertical slice, and
> observation selection/delivery/lifecycle slice are implemented. See
> `RESULTS.md`, `RESULTS-OBSERVABLE-OPERABLE.md`, and
> `RESULTS-OBSERVATION-REQUESTS.md`. Later scheduler, failure, timestamp,
> storage, and agent-friendliness phases remain specified below. This is not a
> production API proposal.

## North star

The SDK should help developers create composable Components that may expose
Observables, Operables, or both. Those Components should compose in the same
conceptual way whether they run in one process, on one Peer, or across Peers.

The ultimate purpose is multi-robot orchestration. The data plane contributes
three foundations:

1. **Shared frames of reference** — identity, time, space, representation,
   units, and semantics must travel with data and instructions.
2. **Language hygiene** — one term should name one concept, and declarations
   must truthfully describe the SDK payload or instruction at the interface.
3. **Composability** — Components should form typed, inspectable collaborative
   pipes without inventing a new integration path for each application.

In one sentence:

> The SDK provides typed Component interfaces that let Components observe and
> operate one another without ambiguity about identity, meaning, or execution
> context.

## Decisions and terminology

### Peer

A **Peer is an authenticated SDK runtime**. It is the SDK's identity,
authority, networking, Catalog, and runtime boundary.

Identity, credentials, and runtime configuration create a live Peer instance:

```text
PeerIdentity + Credentials + PeerConfig
                    |
                    v
             running Peer instance
                    |
                    v
              hosts Components
```

The same stable Peer identity and configuration may create a new runtime
session after a restart. A Peer is therefore not merely a robot, process, or
configuration object.

### Component

A **Component is a composable unit of behavior hosted by a Peer**. It may read
from outside the SDK, compute from SDK inputs, affect the outside world, or
combine those behaviors.

```text
Peer: robot-7
|- Component: front-camera
|- Component: object-detector
|- Component: battery-monitor
`- Component: left-arm
```

The Peer answers **who is participating and under whose authority**. The
Component answers **what behavior or capability is provided**.

### Component and Component Output identity

A Component and one configured output of that Component have separate
identities:

- **Component ID and Component Manifest hash** identify the stable unit of
  behavior and its declared Observable and Operable interface contract.
- **Output ID and Output Manifest hash** identify one immutable configured
  production contract for one named Component output slot.

```text
Camera Component: front-camera @ component-hash-a
  output slot: frames
    current Output: frames-17 @ output-hash-17
```

Changing a contract-affecting setting such as camera resolution replaces the
Output, not the Camera Component:

```text
front-camera @ component-hash-a                 unchanged
  frames-17 @ output-hash-17, 1920x1080         concluded
  frames-18 @ output-hash-18, 1280x720          current
```

The stable output slot name `frames` may resolve to the current Output. An
observation pinned to `frames-17` never silently becomes an observation of
`frames-18`. A separate follow-current request may cross the replacement only
by reporting the old and new Output identities explicitly.

Products reference the exact Output that produced their observations. A
Product Manifest must therefore name the producer Component ID, Output ID, and
Output Manifest hash. Hashes pin exact manifests for identity and integrity;
consumers compare typed standardized fields, not hashes, for compatibility.

### Observable

An **Observable<T> is a named typed interface through which one Component can
show another Component something**. It defines what can be observed, including
its type and meaning. It does not prescribe how the observing Component must
ask or how the answer must be delivered.

Each Observable declares the questions it knows how to answer. Depending on
the Observable and the data available behind it, those questions might include:

- show me the latest observation;
- show me the first available observation;
- show me every available observation;
- show me the observations from a particular time or time range;
- continue showing me new observations as they become available.

These are examples, not a universal enum that every Observable must implement.
A fresh Component Observable, retained Buffer Product, and concluded Episode
Product will support different questions. Being Observable does not itself
imply retained history or a live subscription.

In this experiment, an Observable remains a **Component interface**. A Buffer
or Episode is a **Product**, not a Component. Products may reuse the same typed
selection vocabulary through a retained-product access API, but that does not
turn the Product into an Observable Component interface. A Component
Observable may use a Product as a backing store when it deliberately exposes
retained access.

```text
Camera.frames             Observable<VideoFrame>
BatteryMonitor.charge     Observable<GaugeObservation>
Arm.joint_states          Observable<JointState>
```

The observing Component may be hosted by the same Peer or another Peer.
Observable does not mean public to the whole cluster; exposure and
authorization determine which Components may observe it.

### Operable

An **Operable<I, R> is a named typed interface through which another Component
may instruct a Component to configure itself or execute behavior**. `I` is the
instruction type and `R` is an acknowledgement or result type when one exists.

```text
Camera.set_resolution     Operable<SetResolution, AppliedResolution>
Camera.take_photo         Operable<CaptureRequest, PhotoReference>
Display.present           Operable<DisplayFrame, Presented>
Arm.move_joints           Operable<JointTargets, MotionAccepted>
```

An internal method is not automatically an Operable. The interface must be
deliberately exposed to another Component. A camera whose application alone
can call a private `set_resolution` method is not Operable through that method.
It becomes Operable when `set_resolution` is exposed as a typed Component
interface.

An Operable is also not synonymous with every `InputPort<T>`. An input port may
be ordinary implementation plumbing for a processing graph. An Operable
expresses agency: another Component can intentionally cause configuration or
execution.

### Components may expose either or both

Observable and Operable are not mutually exclusive Component kinds:

```yaml
component: front_camera

observables:
  - name: frames
    datatype: video_frame

operables:
  - name: set_resolution
    instruction: camera_resolution
    result: applied_camera_resolution
  - name: start_capture
    instruction: start_capture
  - name: stop_capture
    instruction: stop_capture
```

Joint encoder readings are Observable. Joint control is Operable. That does
not imply that one robot-arm Component should expose both. A clean design may
use a joint-state Sensor Component and a separate joint-control Actuator
Component, even when both ultimately refer to the same physical arm.

Observable and Operable describe interfaces, not a requirement to combine
sensing and actuation. Prefer separate Components when observation and control
have different authority, safety, lifecycle, failure, or replacement
boundaries. Combine them only when one Component must own an inseparable device
state machine or provide atomic behavior that would become incorrect across a
Component boundary. Separate Components must not independently race to own the
same hardware driver; they may share a lower-level device owner that is not
itself part of the public Component model.

### Interface meaning is independent of reachability

Whether an interaction crosses a Peer boundary does not redefine the
interface:

```text
Component A --asks to observe--> Component B Observable<T>
Component A --instructs------> Component B Operable<I, R>
```

Either relationship may be local or remote. The implementation path may be
very different, but the application-level meaning should remain stable.

Each exposed interface has a reachability policy:

```rust
enum Exposure {
    Local,
    Cluster,
}
```

This is only candidate vocabulary. The important separation is:

- **Observable versus Operable** says what the interface means.
- **Local versus cluster exposure** says where it may be reached.
- **Authorization** says which caller may use it.
- **Transport** says how an allowed interaction is delivered.

Only cluster-exposed interfaces appear in the Cluster Catalog. The Catalog
must enumerate the actual Observable and Operable contracts; booleans such as
`observable: true` or `operable: true` are insufficient.

### Kind, datatype, schema, meaning, and unit are separate

The Output kind helps discovery. The port datatype and schema provide type
safety and shape. Semantic fields explain what the value means. In this
experiment, `kind`, `datatype`, `schema`, `observes`, and `unit` belong to the
immutable Output Manifest rather than the stable Component Manifest. This
prevents a Component with several differently shaped outputs from being
described by one misleading producer kind.

The agreed Gauge example is:

```yaml
kind: gauge
observes: battery_state_of_charge
datatype: float64
unit: percent
```

## What the first experiment established

The first implementation is in PR
[#361](https://github.com/aukilabs/auki-sdk/pull/361). It tested static and
runtime-connectable typed ports, explicit delivery policies, shared immutable
payloads, Buffers, StreamPumps, Episode promotion, and an experimental chunk
builder.

### Supported findings

1. **Typed named interfaces make graphs legible.** Compile-time payload types
   rejected incompatible connections, and semantic names made direction clear.
2. **A static local path can be essentially free.** The concrete inline path
   matched the handwritten call in the first benchmark and compiled without an
   indirect per-payload call.
3. **Dynamic composition has real overhead.** Runtime port dispatch was about
   40 ns per small publication in the recorded run, versus about 2.2 ns for the
   static and direct paths.
4. **Shared immutable payload ownership works.** Eight consumers, a Buffer, a
   receiving Buffer, and an Episode could share payload storage without eight
   byte-for-byte copies.
5. **Delivery policy must be explicit.** The prototype's `Every` and `Latest`
   policies make different delivery promises and must report backpressure,
   replacement, disconnection, and gaps truthfully. The revised vocabulary
   distinguishes those policies from observation-selection questions.
6. **Buffers can be bounded and independently useful.** They provide recent
   history, cursors, retained ranges, gap evidence, and Episode promotion.
7. **A Buffer has not earned a mandatory place on every path.** The prototype
   Buffer and Buffer-backed pump were materially slower than narrower control
   paths.
8. **Chunks remain an implementation experiment.** No measured result yet
   justifies making immutable chunks the semantic definition or required
   storage unit of a Buffer.

### First recorded benchmark

The following was one unpinned local run and is evidence, not a universal
performance claim:

| Case | Nanoseconds per publication |
|---|---:|
| Handwritten direct call | 2.23 |
| Static inline connection | 2.19 |
| Dynamic inline connection | 40.05 |
| One-entry Buffer append | 78.33 |
| Current `CameraFrameHub`, eight stalled subscribers | 21.77 |
| Direct latest pump | 121.66 |
| One-entry Buffer then pump | 181.81 |

The first experiment therefore supports one public semantic model with more
than one optimized implementation path. It does **not** support forcing every
local connection or every remote stream through one runtime mechanism.

## Revised experiment question

The original question was whether the SDK should use one typed dataflow
runtime for local production, processing, retention, and delivery. That is now
too coarse.

The revised question is:

> Can the SDK present one precise Component model—Observable and Operable
> interfaces—while selecting distinct static, dynamic, retained, and
> transported implementations without changing application semantics?

The experiment must determine whether:

1. Components can expose typed Observables and Operables without knowing
   whether the observing or instructing Components are local or remote.
2. Local static composition can remain compiler-optimizable.
3. Dynamic local and transport-backed handles can preserve the same semantics
   with measurable, bounded overhead.
4. Buffers can remain optional observers used for retention rather than a
   compulsory hop.
5. Cluster Catalog entries can truthfully describe only the interfaces that
   are deliberately cluster-exposed.
6. A coding agent can assemble a valid graph without confusing producer kind,
   datatype, schema, meaning, retention, or reachability.

## Implementation prompt

Continue the isolated Rust experiment in phases by evolving
`experiments/typed-dataflow/`. Do not modify the SDK's production networking,
Manager, heartbeat, Domain, Registry, Log, or streaming paths.

The goal is not to preserve the first prototype API. Keep its benchmark and
correctness evidence as baselines, but replace assumptions that no longer fit
the revised model.

### Required public model

Prototype the smallest coherent form of these concepts:

```rust
Component
ComponentManifest
ComponentOutput<T>
OutputManifest
ProductManifest
Observable<T>
Operable<I, R>
Observation<T>
ObservationHandle<T>
RetainedProduct<T>
FiniteObservations<T>
Invocation
Exposure // local or cluster
```

The exact Rust representation is part of the experiment. Do not force all
concepts through trait objects or heap allocation merely to make their names
uniform.

An Observable must declare:

- Component identity and semantic interface name;
- observation datatype;
- the observation requests it supports;
- delivery behavior for requests that may return more than one observation;
- local or cluster exposure;
- enough metadata to associate observations with their source identity,
  clock, spatial frame when applicable, schema, semantics, and unit.

Do not encode “Observable” as shorthand for “live subscription.” Selection and
delivery are separate concerns. For example, “show me Tuesday” selects a time
range; whether the selected observations must all be delivered or may be
coalesced is a different policy.

Do not require one universal request enum or one universal return shape. The
Phase 1 API must make these minimum operations explicit:

```rust,ignore
latest_existing() -> Result<Option<Observation<T>>, ObservationError>
time_range(...)   -> Result<FiniteObservations<T>, ObservationError>
follow_new(...)   -> Result<ObservationHandle<T>, ObservationError>
```

`latest_existing` may return only an observation that already exists. It must
not instruct a sensor to manufacture a new sample. A fresh-only Component
Observable therefore supports `follow_new` but not `latest_existing` or
`time_range` unless it deliberately has retained backing storage.

`ObservationHandle<T>` owns one continuing observation relationship. It must
expose an inspectable status (`active`, `completed`, `reconfigured`, `failed`,
or `cancelled`), explicit cancellation, delivery statistics, and a terminal
reason. A pinned handle that encounters an Output replacement becomes
`reconfigured` and names the replacement Output. A follow-current handle stays
active but delivers the explicit transition.

A Buffer or Episode exposes retained-product access rather than pretending to
be a Component. Its Product Manifest declares only the finite selection
operations that its access implementation can currently serve.

An Operable must declare:

- Component identity and semantic interface name;
- typed instruction and result or acknowledgement;
- local or cluster exposure;
- invocation identity and caller context;
- explicit completion, rejection, cancellation, and error behavior.

Do not treat ordinary private methods, lifecycle hooks, or all processing
inputs as Operables.

### Local and remote-shaped paths

Implement two in-memory Peer runtimes. They are test fixtures, not a new
network stack. A Peer fixture should supply identity, Component hosting,
Catalog projection, and a bounded transport adapter. Authentication may be
represented by explicit caller identity and an allow/deny policy; do not build
cryptography or integrate the production authorization system.

Exercise the same interface semantics in four cases:

```text
local Component -> local Observable request and response
remote Component -> transported Observable request and response
local Component -> local Operable invocation
remote Component -> transported Operable invocation
```

Local paths must not serialize. The semantic in-memory transport fixture from
the identity slice is not evidence about network copying or performance.
Phase 1 must also include a serialized in-memory adapter that actually encodes
and decodes transported requests and observations, counts encoded bytes, and
demonstrates that unavoidable transport copies are distinct from local
zero-copy fan-out. Transport details must not leak into Component
implementations.

### Demonstration Components

Build this minimum graph:

```text
Peer A
  Camera Component
    Observable<VideoFrame>: frames
    Operable<SetResolution, AppliedResolution>: set_resolution
    private local method: reset_driver

  MeanBrightness Component
    asks Camera.frames to show each new frame locally
    Observable<GaugeObservation>: level

Peer B
  Preview Component
    asks Camera.frames to continue showing new frames through the transport adapter

  CameraController Component
    invokes Camera.set_resolution through the transport adapter
```

Use truthful Gauge metadata for the derived value, for example:

```yaml
kind: gauge
observes: image_mean_luminance
datatype: float64
unit: percent
```

The private `reset_driver` method must not become an Operable or appear in the
Catalog. Add a local-only Operable as well and prove that a Component on Peer A
can invoke it while Peer B cannot discover or invoke it.

### Observable selection and delivery

Model these as two independent decisions:

1. **Selection:** which observations answer the question—for example latest
   existing, first available, all available, a time range, or new observations
   from now.
2. **Delivery:** what happens if the answer contains multiple observations and
   the observer cannot keep up.

Retain the first experiment's explicit delivery guarantees, but use names that
do not confuse them with a “show me the latest” selection request:

- **EverySelected:** preserve accepted selected observations in order. A full
  bounded path must backpressure, reject, or disconnect explicitly; it may not
  silently drop.
- **CoalesceLatest:** retain at most the newest pending selected observation.
  Replacements must be counted and observable.

The exact API names remain experimental. The semantic distinction does not.
Requesting the latest observation should normally return one value; following
new observations with `CoalesceLatest` is a continuing relationship that may
skip intermediate values under pressure.

The producer must create one immutable payload representation that local owning
observers share. Adding eight local observers must not copy large payload bytes
eight times. A serialized remote path is expected to encode and copy bytes; it
must report that work instead of using shared in-process ownership as false
evidence of network zero-copy.

### Operable invocation

Test at least:

- successful local and transported invocation;
- typed acknowledgement or result;
- invalid instruction types rejected at compile time on typed handles;
- unauthorized caller rejection;
- unknown or unexposed Operable rejection;
- cancellation or deadline behavior;
- concurrent instructions with a declared ordering policy;
- errors attributed to the target Operable rather than reported as observation
  loss.

Changing camera resolution changes the configured production contract, but it
does not replace the Camera Component. Applying `set_resolution` must conclude
the current `frames` Output and create a new immutable Output with a new Output
ID, Output Manifest, and Output Manifest hash. The Component ID and Component
Manifest hash remain stable.

The `AppliedResolution` result must identify the replacement Output and the
exact sequence or time boundary at which it becomes effective. Frames before
that boundary reference the old Output Manifest hash; frames after it reference
the new Output Manifest hash. The old Observable must not silently begin
emitting observations governed by a different Output contract.

The experiment must choose and document how an active observation relationship
crosses this boundary. The simplest correct behavior is to conclude the old
relationship with an explicit `Reconfigured` reason and require the observer
to resolve and observe the replacement Output. A distinct follow-current
request may automatically rebind only if the transition and new Output
identity remain explicit to the observer.

Any Buffer or Episode whose Product Manifest names the old producer Output
must continue to describe only observations produced under that contract. The
experiment should roll to a new Buffer when the replacement Output becomes
effective rather than mixing two production contracts into one deceptively
homogeneous product.

### Optional retention

A Buffer is an optional Product that observes and retains an Observable:

```text
Camera.frames Observable
  |- direct local observer
  |- transport-backed observer
  `- Buffer
       `- Episode promotion
```

Preserve the first prototype's bounded retention, cursor, gap, shared-payload,
and Episode-promotion correctness tests. Retained Product access may answer
latest-existing and time-range questions that the fresh producer Observable
cannot. A Component may deliberately route retained requests to that Product,
but the Product does not become a Component. Do not require direct observers
or remote delivery to pass through a Buffer.

Keep the direct-pump and Buffer-backed-pump benchmark as a decision control.
Do not add a chunk store unless a specific measured retention, replay,
encoding, persistence, or promotion problem motivates it.

### Catalog projection

Generate a minimal Catalog view from each Peer fixture. It must:

- list cluster-exposed Observables by Component and interface name;
- state which observation requests each Observable currently supports;
- include the current Output ID and Output Manifest for each exposed output
  slot;
- list Product Manifests separately and state which retained-data selections
  their access implementation currently supports;
- list cluster-exposed Operables with instruction and result contracts;
- omit private and local-only interfaces;
- distinguish kind, datatype, schema, `observes`, and unit;
- avoid claiming availability for an interface that the transport adapter
  cannot currently serve.

The Catalog is a projection of runtime exposure, not the owner of Component
behavior.

### Correctness gates

Automated tests must prove:

1. A Component may expose only Observables, only Operables, or both.
2. Incompatible typed Observable observation paths do not compile.
3. Incompatible typed Operable instructions do not compile.
4. Local and transported Observable paths preserve the declared selection and
   delivery semantics.
5. Local and transported Operable paths produce equivalent typed results.
6. An Operable invocation can cause a visible change in Component behavior.
7. Private and local-only interfaces do not appear in the remote Catalog.
8. A remote Component cannot invoke a local-only Operable.
9. Caller Peer and Component identity reach the Operable invocation context.
10. Eight observers share one large immutable payload allocation.
11. A stalled dynamic observer cannot create unbounded memory growth.
12. Adding or removing a Buffer does not change Observable payload semantics.
13. Dropping an observation or invocation handle releases its owned runtime
    state.
14. No local static path serializes or performs disk I/O.
15. A returned Component error changes the affected observation or invocation
    relationship to an explicit inspectable state rather than silently ending
    delivery.
16. A panic on an asynchronous path cannot silently kill a worker while its
    handle continues to claim that it is live; the experiment must document
    and test the chosen panic boundary.
17. One failed observer or Operable invocation does not terminate unrelated
    observers or invocations.
18. Buffers handle equal, missing, and out-of-order source timestamps according
    to an explicit policy.
19. Time-range observation requests and duration eviction use a declared time
    basis and never silently mix source time with arrival time.
20. An Episode and Buffer can share sealed retained storage without copying;
    each releases its ownership independently.
21. Evicting an externally backed payload does not recycle or overwrite its
    storage while an observer, Buffer, Episode, or transport still holds a
    lease.
22. External-memory accounting contributes truthful retained bytes to hard
    Buffer limits.
23. A bounded shared scheduler can cancel and drain observation work without
    leaking tasks or requiring one OS thread per observer.
24. Applying `set_resolution` preserves the Component ID and Component
    Manifest hash while creating a replacement Output ID and Output Manifest
    hash.
25. Observations on either side of the reconfiguration boundary reference the
    correct immutable Output Manifest.
26. An existing observer receives an explicit reconfiguration transition and
    cannot unknowingly consume observations governed by the replacement
    contract.
27. A Buffer whose Product Manifest references the old Output Manifest does
    not silently retain observations produced under the new Output Manifest.
28. A fresh-only Observable does not advertise or accept latest-existing or
    time-range requests.
29. A Buffer Product answers latest-existing and time-range requests without
    appearing as a Component in the Catalog.
30. A serialized transport path preserves observation semantics while
    reporting encoded bytes and not claiming local allocation identity.

## Phased execution

This document is the test program, not one indivisible implementation change.
Each phase must preserve the completed evidence and may stop independently if
its result invalidates the model.

### Completed foundation: typed dataflow

Typed ports, static and dynamic dispatch, explicit delivery policies, bounded
Buffers, shared local payload ownership, StreamPumps, Episode promotion, and
the initial performance controls are implemented in PR #361.

### Completed foundation: Component and Output identity

Stable Component identity, immutable configured Output identity, Output-bound
Buffer Products, Catalog projection, typed Operable authorization, and pinned
versus follow-current reconfiguration are implemented on
`codex/observable-operable-data-plane`.

### Completed Phase 1: observation selection, delivery, and lifecycle

The branch `codex/observation-requests-lifecycle` implements explicit
latest-existing, time-range, and follow-new shapes; truthful request
advertisement; retained Product access; inspectable continuing handles;
cancellation; reconfiguration status; and serialized transport semantics.
Correctness gates 1–14 and 24–30 apply where relevant. Its targeted benchmark
measures the extra follow-current dispatch and serialized-copy boundary; the
full benchmark matrix remains Phase 4 work.

### Phase 2: failure, concurrency, and scheduling

Implement explicit Component errors, panic boundaries, Operable deadlines and
cancellation, concurrent instruction ordering, and a bounded shared scheduler.
Correctness gates 15–17 and 23 apply.

### Phase 3: time and retained storage

Resolve timestamp policy, duration retention, shared Buffer/Episode storage,
chunk evidence, and external-memory leases and accounting. Correctness gates
18–22 apply.

### Phase 4: evaluation

Run the reproducible performance matrix, stored-Log comparison, and clean-room
agent-friendliness exercise. Produce the final recommendation about the public
semantic model and optimized implementation paths.

### Performance comparisons

Measure these paths independently:

1. handwritten concrete observation callback;
2. static live Observable observation;
3. dynamically connected local live Observable observation;
4. transport-adapter live Observable observation;
5. handwritten direct configuration call;
6. static Operable invocation;
7. dynamically resolved local Operable invocation;
8. transport-adapter Operable invocation;
9. direct Observable-to-pump delivery;
10. Observable-to-Buffer-to-pump delivery.

Report publish or invocation cost, end-to-end p50 and p99 latency, allocation
count, bytes copied, CPU time, peak retained memory, and all replacement,
backpressure, rejection, cancellation, and gap counts.

The static paths should again be inspected for indirect calls in the hot loop.
Dynamic and transported paths are allowed to cost more, but their overhead
must be reported rather than hidden behind a blended benchmark.

## Required follow-up coverage from the first results

Every item listed as **Deliberately unresolved** in the first experiment's
`RESULTS.md` is required work for the complete phased program. An item may remain
architecturally undecided, but it may not remain untested or unmeasured without
an explicit blocking reason in the final report.

### Measurement harness

Replace the one-off timing run with a reproducible harness that:

- records host, operating system, compiler, target, optimization profile, and
  relevant feature flags;
- pins the benchmark to a CPU when the platform supports it, and records when
  pinning is unavailable;
- includes warmup and repeated samples rather than reporting one run;
- prevents the compiler from removing payload construction or consumption;
- counts allocations and deallocations per observation or invocation;
- instruments payload-byte copies separately from cheap handle clones;
- reports p50 and p99 publish-to-observer or invoke-to-result latency;
- reports CPU time and peak retained memory;
- reports throughput together with every replacement, rejection,
  backpressure, overrun, and gap count.

Run the matrix with small scalar observations, medium audio blocks, and large
camera-frame handles. Separate producer-call latency from end-to-end latency;
neither substitutes for the other.

### Current stored-Log detector comparison

Add a separate end-to-end comparison with the current stored-Log detector
input path. Feed logically equivalent camera observations through:

```text
decoded frame -> direct typed Component path -> detector
decoded frame -> Buffer path -> detector
stored Log -> read -> decode -> current detector path
```

Report storage read, decoding, dispatch, and detector execution costs
separately. Include warm-cache and cold-cache results where the platform makes
that distinction measurable. Do not blend disk I/O and decoding into a number
presented as typed-dispatch overhead.

This benchmark is a reference comparison. It does not authorize changes to the
production stored-Log path.

### Chunk and Episode evidence

Extend the chunk sidecar only far enough to answer whether chunks solve a
measured problem. Test:

- a Buffer and promoted Episode sharing the same sealed-chunk storage identity;
- Buffer eviction while the Episode continues retaining the chunk;
- final release after both data products relinquish the chunk;
- promotion while an open partial chunk exists;
- replay, encoding, persistence, memory, and promotion cost against per-payload
  retained storage;
- live observation latency remaining independent of chunk sealing.

The final report must identify a measured win that justifies chunk complexity
or recommend removing chunks from the next design. Lifecycle correctness alone
is not sufficient evidence.

### Scheduler alternatives

The first prototype used one OS thread per `BufferReader` and queued
connection. Compare that baseline with at least one bounded shared worker or
async scheduler implementation.

Run one, eight, 64, and 256 simultaneous queued observation or instruction
relationships, including one deliberately blocked observer. Measure:

- OS thread and task count;
- throughput and p50/p99 latency;
- fairness between observers;
- memory per relationship;
- cancellation and shutdown completion time;
- whether a blocked observer starves unrelated work.

The purpose is not to choose the SDK's final global scheduler. It is to prove
that the semantic model does not require one thread per observer and to expose
the cost of the alternative.

### Component errors and panics

Give Observable delivery and Operable invocation explicit failure state. Test
at least:

- a recoverable error returned by an inline observer;
- a recoverable error returned by an asynchronous observer;
- an Observable producer reporting a terminal Component error;
- an Operable rejecting an otherwise well-typed instruction;
- a panic on the inline path;
- a panic on an asynchronous worker;
- inspection of the failed relationship after the failure;
- isolation of unrelated observers and Operables;
- cancellation, cleanup, and payload release after failure.

The experiment need not promise recovery from every panic. It must prevent a
handle from claiming healthy delivery after its worker has silently died, and
it must document whether panic propagation, containment, or process abort is
the supported boundary for each path.

### Timestamp and retention correctness

Sequence order and time order are not equivalent. Test Buffers and time-based
observation requests with:

- strictly increasing source timestamps;
- equal timestamps;
- observations arriving out of timestamp order;
- a source clock moving backward;
- observations without a usable source timestamp;
- source timestamps from the wrong declared clock;
- arrival time disagreeing materially with source time.

The Buffer must declare whether duration retention and time-range selection use
source time, arrival time, or another normalized time basis. Unsupported or
invalid timestamp cases must be rejected, quarantined, or handled by another
explicit policy; they must not silently corrupt the advertised retained range.

### GPU, DMA, pooled, and other external storage

Add a payload fixture whose bytes are owned outside ordinary `Arc<T>` heap
storage. A simulated external allocation with explicit lease and release hooks
is acceptable if real GPU or DMA hardware is unavailable.

Prove that:

- fan-out clones a handle or lease rather than copying payload bytes;
- Buffer eviction releases only the Buffer's lease;
- Episode promotion and transport can retain independent leases;
- the backing allocation is released or returned to its pool exactly once;
- a producer cannot overwrite pooled storage while any observer retains it;
- retained-byte accounting reflects the external allocation rather than
  `size_of::<T>()`;
- an unavoidable GPU-to-CPU readback is explicit and measured as a copy.

Run the large-payload fan-out and retention benchmarks with both ordinary heap
storage and the external-storage fixture.

### Agent-friendliness check

Give only the public experiment documentation to a coding agent and ask it to
add:

1. a Microphone Component with an `audio` Observable;
2. a local-only `set_gain` Operable;
3. a cluster-exposed `start_capture` Operable;
4. a Volume Component that observes `audio` and emits a Gauge Observable;
5. a deliberately invalid audio-to-video observation path;
6. a deliberately invalid remote invocation of `set_gain`.

Record whether the agent:

- distinguishes Observable delivery from Operable invocation;
- keeps local versus cluster exposure separate from interface meaning;
- uses typed contracts rather than stringly typed payloads;
- describes the final emitted payload truthfully;
- uses `kind`, `datatype`, `schema`, `observes`, and `unit` consistently;
- bypasses the SDK abstractions or invents undeclared Catalog entries.

This exercise must actually be run. Preserve the initial prompt, generated
patch, compiler and test output, corrections requested from the agent, and
final result. Do not count an implementation written by an author who already
read the prototype internals as this test.

### Deliverables

The complete phased experiment ends with:

1. the isolated prototype code;
2. the two-Peer demonstration;
3. compile-fail and runtime correctness tests;
4. reproducible benchmarks, environment metadata, and raw samples;
5. allocation, copy, latency, CPU, and retained-memory measurements;
6. the separate current stored-Log detector comparison with staged costs;
7. the per-payload versus shared-chunk result and recommendation;
8. the one-thread-per-observer versus bounded-scheduler result;
9. documented error, panic, timestamp, and retention policies;
10. the external-storage ownership and accounting tests;
11. an example Catalog projection;
12. the complete agent-friendliness test artifact and observations;
13. a comparison with the first experiment's measurements;
14. a decision about the public semantic model and required optimized paths;
15. explicit unresolved questions before production networking or Catalog
    work.

## Stop conditions

Stop and reassess instead of expanding the prototype if:

- Observable and Operable cannot be defined without transport-specific
  behavior leaking into Components;
- local static composition requires serialization, allocation, or indirect
  dispatch in the hot path;
- Operable becomes indistinguishable from every input port or public method;
- cluster exposure cannot be represented separately from interface meaning;
- payload fan-out requires a byte copy per observer;
- a stalled observer or instruction can cause unbounded memory growth;
- Catalog generation requires Components to lie about availability;
- the experiment begins rebuilding Manager, heartbeat, membership, or
  production networking logic.

The experiment exists to test whether this vocabulary deserves to organize
the SDK. It must remain easy to reject or revise.
