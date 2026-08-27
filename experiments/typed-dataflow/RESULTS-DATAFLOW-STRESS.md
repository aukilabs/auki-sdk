# Observation failure and shared scheduling: Phase 2A results

Date: 2026-08-28

Branch: `codex/dataflow-stress-lifecycle`

## Result

The semantic model does not require one OS thread per observation
relationship. A fixed four-worker scheduler served 256 simultaneous
relationships, isolated one deliberately blocked observer, preserved the
declared `EverySelected` and `CoalesceLatest` behavior, and shut down without
leaking pending payload ownership.

The scheduler is not a free performance win. At one relationship, the original
dedicated worker was faster. At 64 and 256 relationships, the shared scheduler
made producer publication much cheaper and achieved substantially higher
aggregate delivery throughput, but queued observations experienced
millisecond-scale latency. The prototype deliberately schedules only one
observation per relationship per turn; that improves fairness while increasing
queueing and scheduling overhead.

Returned observer errors, observer panics, and producer-reported terminal
failure now become explicit relationship states. They no longer leave an
`ObservationHandle` claiming to be active after delivery has stopped.

No production networking, Manager, heartbeat, Domain, Registry, Log, or
streaming implementation changed.

## Implemented behavior

- `InputPort::try_new` lets a Component return a recoverable error.
- `InputPort` contains callback panics with `catch_unwind`; both returned errors
  and panics close the affected connection and retain an inspectable reason.
- `PublishReport` and `ConnectionStats` distinguish failure from ordinary
  disconnection.
- `ObservationHandle::status` reflects asynchronous connection failure.
- `ObservationEvent::Failed` carries a terminal failure reported by a producer.
- the Camera fixture stops publishing after terminal Output failure, and its
  Buffer Product closes while preserving observations already retained.
- `SharedScheduler` owns a fixed number of workers. `SharedDispatcher` is the
  lightweight capability retained by connections.
- a shared connection has either a bounded `EverySelected` queue or a one-slot
  `CoalesceLatest` queue. It may have one running drain and at most one queued
  drain task; payload backlog remains bounded by its declared delivery policy.
- each scheduled turn handles one observation before yielding to the pool.
- the serialized in-memory transport can use the shared scheduler without
  pretending local allocation identity survives serialization.

## Correctness evidence

Ten new integration tests prove:

1. 256 relationships use four named workers and continue serving 255 observers
   while one callback is blocked;
2. shared `EverySelected` preserves every accepted value in order while shared
   `CoalesceLatest` reports and performs replacement;
3. a returned observer error becomes a failed handle;
4. an observer panic becomes a failed handle without killing its scheduler
   worker or an unrelated healthy observer;
5. a producer-reported terminal failure fails the handle, rejects subsequent
   publication, and closes the corresponding Buffer Product;
6. a payload lease remains valid after Buffer eviction and releases after its
   final owner drops;
7. cancelling a shared relationship releases its queued payload ownership;
8. Camera-to-Buffer and Camera-to-local-Component paths share payload storage,
   while the serialized remote path has a distinct allocation and byte counts;
9. the original threaded asynchronous path also reports callback failure;
10. returned errors and panics on the inline path are contained and
    inspectable.

The previous 34 integration tests continue to pass.

## Targeted benchmark

One release run used five samples. Each row is the sample with the median
producer publication cost. Queues were sized to retain the full run, so this
measures scheduling rather than intentional loss. CPU affinity was not pinned.

- compiler: `rustc 1.94.1 (e408947bf 2026-03-25)`
- target: `aarch64-macos`
- shared worker count: 4

| Relationships | Topology | OS workers by construction | Iterations | Median publish ns | Drain after publish ns | Deliveries/s | p50 latency ns | p99 latency ns | Shutdown ns |
|---:|---|---:|---:|---:|---:|---:|---:|---:|---:|
| 1 | Thread per relationship | 1 | 20,000 | 241.3 | 500,167 | 3,754,311 | 1,104,667 | 1,835,666 | 39,542 |
| 1 | Shared scheduler | 4 | 20,000 | 269.8 | 4,564,125 | 2,007,864 | 4,488,917 | 4,665,292 | 69,500 |
| 8 | Thread per relationship | 8 | 5,000 | 6,419.3 | 76,583 | 1,243,274 | 22,792 | 70,167 | 186,375 |
| 8 | Shared scheduler | 4 | 5,000 | 4,057.4 | 218,792 | 1,950,656 | 106,500 | 421,125 | 74,708 |
| 64 | Thread per relationship | 64 | 1,000 | 171,006.2 | 1,041 | 374,253 | 87,917 | 225,750 | 1,442,583 |
| 64 | Shared scheduler | 4 | 1,000 | 12,309.8 | 14,394,208 | 2,396,641 | 6,886,958 | 14,195,500 | 67,250 |
| 256 | Thread per relationship | 256 | 250 | 791,259.3 | 2,167 | 323,531 | 396,500 | 930,167 | 5,824,709 |
| 256 | Shared scheduler | 4 | 250 | 20,819.0 | 17,532,500 | 2,814,744 | 7,522,041 | 17,301,875 | 72,583 |

With one deliberately blocked observer, every other observer completed before
the gate was released in both topologies. At 256 relationships, those fast
observers completed in about 987 microseconds on the threaded path and 171
microseconds on the shared path in this run.

## Interpretation

The shared scheduler removes the catastrophic growth in OS threads and
producer publication cost at high relationship counts. At 256 relationships,
producer publication was about 38 times cheaper and aggregate delivery
throughput was about 8.7 times higher than the thread-per-relationship path in
this run. Shared shutdown was also much faster because it did not join 256
dedicated workers.

Those improvements move work into bounded queues. The p50 latency at 256
relationships was about 7.5 ms rather than 0.4 ms. The dedicated-worker path's
low observation latency partly reflects its very slow producer call: the
producer spent roughly 0.8 ms publishing each value, so deliveries were spread
out before publication completed. Producer latency, aggregate throughput, and
observation latency must therefore remain separate metrics.

The one-item scheduling quantum is defensible for a fairness probe, but it is
unlikely to be the final policy. A small adaptive quantum or work-stealing
executor may reduce queueing without letting one busy relationship monopolize
a worker. This experiment proves architectural possibility, not executor
selection.

## Deliberate limitations

- The task channel itself is not capacity-limited. Its task count is
  structurally bounded by live connections, but a production scheduler should
  encode and test that invariant directly.
- A blocked callback still occupies one worker. With every worker blocked, the
  pool cannot make progress; Components need cooperative or deadline-aware
  behavior if that must be prevented.
- Component callback panics are contained and attributed. A panic caused by an
  internal scheduler bug is counted globally but is not yet attributed to an
  individual relationship.
- The benchmark does not pin CPU affinity or report CPU time, allocations,
  peak memory, or scheduler memory per relationship. Those remain Phase 4
  requirements.
- The benchmark uses small in-memory payload handles. Audio blocks, camera
  handles, GPU/DMA leases, and production serialization require the Phase 4
  matrix.
- Operable deadlines, cancellation, and concurrent instruction ordering remain
  Phase 2B.
- `BufferReader` and `StreamPump` still use dedicated threads. This slice proves
  shared scheduling for observation relationships; it does not yet unify every
  background activity under one executor.

## Recommendation

Retain the semantic split between selection, delivery policy, and execution
policy. Do not expose “threaded” or “shared scheduler” as part of an Observable
contract. Use the fixed-pool result as evidence that the SDK API can remain
composable without requiring one thread per relationship, while treating the
actual executor and fairness quantum as replaceable runtime policy.
