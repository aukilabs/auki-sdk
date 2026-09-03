# Component execution

Detectors and Mappers are application-controlled processing Components. They
consume SDK Products and produce SDK Products. A running Component is not
itself a durable Product, but its immutable Manifest and currently configured
inputs/outputs may be projected in a Catalog. Applications may explicitly
export its Products or cluster-visible Operables through
`auki-component-protocol`.

For example, an application can ask the SDK to run `QRDetectorBeta2` against a
particular RGB camera stream once per second. The detector implementation owns
QR recognition. The application owns the choice of stream and cadence. The SDK
runner owns compatibility checks, scheduling, timestamps, provenance,
backpressure, cancellation, and writing the Detection Log. Peers consume that
Detection Log without needing the detector code.

## Four responsibilities

| Layer | Owns |
|---|---|
| Component definition | Stable implementation identity, configuration, accepted input contracts, declared output types |
| Component implementation | Domain computation over one valid input sample |
| SDK runner | Contract validation, cadence, provenance, bounded scheduling, isolation, lifecycle, counters, output plumbing |
| Application | Which implementation runs, concrete Resource inputs and output, live or replay mode, cadence, start/stop policy, resource budget |

Catalog-query and automatic source-selection helpers are conveniences. They may
filter for compatible Resources and return one unambiguous candidate, but they
must fail closed on ambiguity. They do not move policy ownership from the
application into a Mapper or Detector.

## Resource boundary

The durable or streamable result is the interoperability boundary:

```text
Sensor Log + Pose Log -> Mapper instance -> Map Log
Camera Sensor Log     -> Detector instance -> Detection Log
```

Other peers discover and consume the output Product. A remote Product mirror
becomes an ordinary typed input on the consuming peer while preserving its
source Manifest, sequence, and timestamp. Peers do not receive the producer's
in-process object. Remote Operable invocation addresses an explicitly exported
control interface on an already-running Component; starting or placing new
Components would remain a separate application service.

## Live mode

Live streams can arrive faster than an implementation can process them. A live
runner must therefore:

1. keep network and control-plane ingestion on the async runtime;
2. execute CPU-heavy or synchronously blocking component work on a dedicated
   blocking worker;
3. bound every queue between ingestion, processing, and output;
4. declare its overload policy and expose drop counters;
5. respond to cancellation without waiting for queued stale work.

For camera detection and voxel mapping, the default overload policy is a
single pending sample: while the worker is busy, a newer ready-to-process sample
replaces the older one. This is **latest-wins**. It bounds memory and latency and
prevents a slow component from starving authenticated protocol, stream, or UI
tasks. The item currently executing is allowed to finish; cancellation discards
pending work and prevents another item from starting.

Latest-wins is applied only after any multi-stream inputs are ready. A voxel
Mapper still buffers point clouds waiting for bracketing poses under a separate
explicit bound. Once a point cloud has an aligned pose, the resulting mapping
job enters the latest-wins worker queue.

An API being `async` does not make its implementation non-blocking. Output sinks
must return quickly to the runtime and move compression, compaction, encoding,
or blocking disk work to an appropriate worker as well.

## Replay mode

Recorded Sensor Log replay has different semantics. It processes accepted
samples in source timestamp order and does not silently discard work merely
because processing is slower than capture. Its bounds are supplied by the
finite recording and explicit cancellation. A caller that wants sampled replay
chooses a cadence or filter deliberately.

Live and replay paths may share component implementations, validation, cadence,
provenance, and output encoding. They must not accidentally share overload
semantics.

## Operational contract

A production runner should make these values observable:

- received input samples;
- samples rejected by cadence or compatibility;
- samples replaced or dropped at each bounded queue;
- outputs successfully written;
- current state and terminal error;
- selected input and output Resource identities.

Shutdown has an observed form that waits for workers and returns their terminal
result. Dropping a runner requests best-effort cancellation, but applications
should use the observed shutdown path when correctness or diagnostics matter.

## Integration checklist

- The application starts work only when it needs the output (for example, a
  visible Park voxel tile), and stops it when demand disappears.
- A shared subscription fans out frames through a bounded hub rather than
  opening duplicate streams or letting one consumer block another.
- Heavy third-party code never runs on an SDK async runtime worker.
- Queue capacity and overload behavior are explicit and tested.
- Logs retain exact source, sensor, clock, detector/Map, and writer provenance.
- Replay tests prove ordered exhaustive behavior; live tests prove bounded
  latest-wins behavior and runtime responsiveness.
