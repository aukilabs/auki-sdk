# Observable/Operable vertical slice: first results

Date: 2026-08-28

Branch: `codex/observable-operable-data-plane`

## Result

The Component-versus-Output identity split is coherent enough to continue
testing.

The prototype demonstrates one stable Camera Component whose configured
`frames` Output can be replaced without mutating the Component Manifest or
silently changing the contract observed by consumers.

```text
Camera Component @ component hash A
  Operable: set_resolution

  frames-1 @ output hash 1
      | explicit Reconfigured transition
      v
  frames-2 @ output hash 2
```

This resolves the self-replacement problem in the earlier model. The Camera
Operable remains addressable because it belongs to the stable Component. The
configured production contract belongs to an immutable Component Output.

## Implemented behavior

- deterministic experimental SHA-256 hashes for Component, Output, and Product
  Manifests;
- stable Component identity and Manifest hash across resolution changes;
- new Output ID and Output Manifest hash for each contract-affecting resolution
  change;
- an Observable pinned to one immutable Output;
- an opt-in follow-current Observable for one stable output slot;
- explicit `Reconfigured` events naming previous and replacement Outputs;
- a typed `set_resolution` Operable with caller context and authorization;
- a local-only Operable omitted from the cluster Catalog;
- a minimal Catalog containing visible Components and Products, with current
  Outputs nested under their Component;
- truthful RGB8 frame validation against the current Output Manifest;
- one Buffer Product per Output contract, rolled at reconfiguration;
- shared immutable frame bytes across observers and the retained Buffer;
- an in-memory transport-shaped adapter that leaves production networking
  untouched.

## Correctness evidence

Eight new integration tests prove:

1. resolution replacement changes Output identity and hash but not Component
   identity or hash;
2. the Catalog resolves the `frames` slot to the replacement Output;
3. a pinned observer receives the old observation and terminal transition but
   never receives replacement-Output observations;
4. a follow-current observer sees old observations, the explicit transition,
   and new observations in order;
5. Buffer Products roll at the Output boundary and never mix producer Output
   Manifests;
6. retained and directly observed camera frames share the same byte storage;
7. a local Operable is neither Catalog-discoverable nor remotely invocable;
8. an unauthorized remote caller cannot reconfigure the Camera;
9. a frame that contradicts the active Output Manifest is rejected;
10. setting the already active resolution is a no-op and does not manufacture a
    replacement Output.
11. the Catalog rejects an Output Manifest that does not pin the registered
    Component Manifest hash.

The first experiment's sixteen integration tests still pass unchanged. The
crate's compile-fail doc test passes, and focused Clippy is warning-free.

## Catalog shape exercised

```text
Catalog
|- Component: front-camera
|  |- Component Manifest @ stable component hash
|  `- current output slot
|     `- frames -> Output Manifest @ current output hash
`- Products
   |- front-camera.frames-1.buffer -> frames-1 Output Manifest
   `- front-camera.frames-2.buffer -> frames-2 Output Manifest
```

The Catalog does not define the Component's behavior. It exposes the Component
and Product Manifests that the runtime deliberately makes discoverable.

## Deliberate limitations

This result is semantic and correctness evidence, not production readiness.

- `InMemoryTransport` is a typed adapter, not serialization or a real network.
- Caller authorization is an explicit allow-list closure, not production auth.
- Observable access implements continuing live observation only. Latest,
  first, all-available, and time-range requests remain unimplemented.
- `Reconfigured` is semantically terminal for a pinned observation, but the
  prototype does not yet expose an `ObservationSession` status object; its
  underlying connection handle remains attached to an inert old Output port.
- Operable invocation is synchronous and has no deadline, cancellation, or
  asynchronous completion model yet.
- Follow-current duplicates event envelopes onto pinned and slot-level ports.
  Payload bytes remain shared, but the dispatch cost has not been benchmarked.
- Reconfiguration is serialized with frame publication by one mutex. Concurrent
  stress and failure injection have not been run.
- Product lifecycle and Catalog garbage collection are not modeled. Concluded
  Buffer Products remain visible.
- Experimental JSON hashing is deterministic for these structs but is not a
  production canonical manifest format.
- Output compatibility still uses typed fields. Hash equality is not treated
  as a compatibility test.
- The broader unresolved benchmark, scheduler, error, timestamp, chunk,
  external-memory, and agent-friendliness matrix in the design document has not
  yet been executed for this slice.

## Recommendation

Keep the separation:

```text
Component ID/hash = stable behavior and interface contract
Output ID/hash    = one immutable configured production contract
Product ID/hash   = one retained or fetchable data product
```

The next experiment should add finite/latest Observable requests and measure
the extra slot-level follow-current dispatch before any of these types move
into a production crate.
