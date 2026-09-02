# Observation requests and lifecycle: Phase 1 results

Date: 2026-08-28; revised 2026-09-02

Branch: `codex/typed-dataflow-complete`

## Result

The Phase 1 distinction is coherent:

- a fresh Component Observable can truthfully offer only continuing
  `FollowNew` access;
- a Buffer remains a separate Product and can offer finite
  `LatestExisting` and clock-qualified `TimeRange` access;
- sharing request vocabulary does not make the Product a Component;
- a continuing relationship can report whether it is active, ended, failed,
  or cancelled instead of leaving an inert connection that still appears
  healthy;
- local fan-out can share immutable payload storage while a serialized path
  performs and counts its unavoidable copy.

No production networking, Manager, heartbeat, Domain, Registry, Log, or
streaming code changed.

## Implemented behavior

- `ObservationDelivery` separates `EverySelected` from `CoalesceLatest` while
  retaining inline and bounded-queue execution choices;
- `ObservationHandle<T>` owns a continuing `FollowNew` relationship and exposes
  status, cancellation, delivery counts, coalescing counts, overruns, and
  serialized-transport counts;
- every handle becomes terminal `Ended(Reconfigured)` and disconnects at a
  configured-contract replacement;
- observing the replacement requires an explicit new subscription;
- the fresh Camera Observable advertises only `FollowNew` and explicitly
  rejects finite retained-data requests;
- `RetainedProduct<T>` provides finite access without implementing
  `Observable<T>` or appearing as a Component;
- Buffer Product Manifests advertise only `LatestExisting` and `TimeRange`, the
  two finite operations implemented in this phase;
- time-range requests name the source clock and reject the wrong clock or
  inverted bounds;
- `kind`, datatype, schema, `observes`, and unit live in the immutable Output
  payload contract; the stable Component Manifest no longer carries a
  misleading producer kind;
- `SerializedInMemoryTransport` JSON-encodes and decodes observations,
  retained-data requests, retained-data responses, instructions, and results,
  while counting messages and bytes;
- a clonable non-owning `ConnectionControl` lets a relationship close itself
  after delivering its terminal notice while the owning handle retains the
  lifecycle responsibility.

## Correctness evidence

Eleven integration tests prove:

1. a fresh Camera advertises and supports `FollowNew` only;
2. the Catalog places observational meaning on the current Output Manifest;
3. a Buffer Product answers latest-existing and time-range queries without
   appearing in the Component Catalog;
4. retained time-range queries validate their clock and bounds;
5. a handle reports terminal reconfiguration, may name the replacement, and
   disconnects;
6. replacement observations require a newly created subscription;
7. subscribing to an already ended configured Output immediately returns its
   terminal notice rather than an inert active handle;
8. dropping a handle stops future delivery;
9. serialized observation delivery preserves values but creates a distinct
   payload allocation and reports encoded bytes;
10. serialized Product queries preserve finite-selection semantics and report
   their transport work;
11. `CoalesceLatest` reports replacements while bounded `EverySelected`
    preserves every accepted observation in order.

The first experiment's sixteen tests and the identity slice's eight tests
continue to pass.

## Targeted benchmark

One release run on the development machine used seven samples after warmup and
reported the median. It is a narrow decision control, not the Phase 4
reproducible benchmark matrix.

| Case | Iterations per sample | Median ns/publication |
|---|---:|---:|
| Camera publication, no observer | 100,000 | 401.69 |
| One local subscription | 100,000 | 418.31 |
| Two local subscriptions to the same Output | 100,000 | 447.40 |
| Serialized in-memory observer | 20,000 | 1,711.45 |

The marginal second local observer was about 29 ns per publication in this
run. The serialized case encoded and decoded an approximately 423-byte JSON
event for each three-byte RGB fixture payload. This demonstrates the copy
boundary; it does not propose JSON as the wire format or estimate real network
latency.

## Deliberate limitations

- A Component Observable with retained backing is not implemented. This phase
  proves fresh rejection and direct Product access; a later slice may test a
  Component deliberately routing finite requests to one of its Products.
- terminal producer failure drives `Ended(Failed)`, while a relationship or
  observer failure drives the distinct relationship `Failed` status;
- The serialized fixture uses a synchronous callback and treats a serde failure
  as a test-fixture invariant violation. It is not a production transport.
- Product access does not yet offer continuing follow behavior. Only the
  operations advertised in its Product Manifest are callable.
- Time-range selection filters timestamps inclusively in source-sequence order.
  Out-of-order, missing, or invalid timestamp policy remains Phase 3.
- Queued delivery still uses the first prototype's one-thread-per-connection
  implementation. The bounded shared scheduler comparison remains Phase 2.
- The benchmark does not count allocations, CPU time, peak memory, or p50/p99
  end-to-end latency. Those remain Phase 4 requirements.
- Experimental JSON manifest hashing remains deterministic for these structs
  but is not a production canonical format.

## Recommendation

Keep the Phase 1 separation:

```text
Component Observable<T>  -> continuing typed capability
Buffer/Episode Product   -> finite retained-data access
ObservationHandle<T>     -> lifecycle of one continuing relationship
```

Proceed to failure and bounded-scheduler work before moving any of these types
into a production crate.
