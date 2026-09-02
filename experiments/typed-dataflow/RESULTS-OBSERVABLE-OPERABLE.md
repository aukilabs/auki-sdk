# Observable/Operable vertical slice: first results

Date: 2026-08-28; revised 2026-09-02

Branch: `codex/typed-dataflow-complete`

## Result

The Component configuration boundary is coherent enough to continue testing,
but a separate Output identity is no longer an accepted conclusion.

The prototype demonstrates one stable Camera Component whose configured
`frames` Output can be replaced without mutating the Component Manifest or
silently changing the contract observed by consumers.

```text
Camera Component @ component hash A
  Operable: set_resolution

  frames-1 @ output hash 1
      | terminal Reconfigured notice
      v
  frames-2 @ output hash 2
```

The Camera Operable remains addressable because it belongs to the stable
Component. A subscription observes one precisely described configuration and
ends when that configuration changes. No subscription survives the boundary.
The fixture still puts the configured production contract in an immutable
Output Manifest, but terminal subscriptions remove the strongest reason for a
stable Output slot and make that identity split provisional.

## Implemented behavior

- deterministic experimental SHA-256 hashes for Component, Output, and Product
  Manifests;
- stable Component identity and Manifest hash across resolution changes;
- new Output ID and Output Manifest hash for each contract-affecting resolution
  change;
- an Observable bound to one immutable configured Output;
- an explicit terminal `Ended(Reconfigured)` notice that may name a
  replacement only as a discovery hint;
- a typed `set_resolution` Operable with caller context and authorization;
- a local-only Operable omitted from the cluster Catalog;
- a minimal Catalog containing visible Components and Products, with current
  Outputs nested under their Component;
- truthful RGB8 frame validation against the current Output Manifest;
- one Buffer Product per configured contract; its subscription ends and its
  Buffer closes at reconfiguration, while replacement retention requires a
  new explicit attachment;
- shared immutable frame bytes across observers and the retained Buffer;
- an in-memory transport-shaped adapter that leaves production networking
  untouched.

## Correctness evidence

Eight new integration tests prove:

1. resolution replacement changes Output identity and hash but not Component
   identity or hash;
2. the Catalog advertises the replacement configuration under the Camera;
3. a subscriber receives the old observation and terminal notice but
   never receives replacement-Output observations;
4. receiving replacement observations requires an explicit new subscription;
5. a Buffer closes at the configuration boundary and recording the replacement
   requires a new explicit Buffer attachment;
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
|  `- current configured Observable
|     `- experimental Output Manifest @ current output hash
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
- `Reconfigured` is terminal and the `ObservationHandle` reports the end
  notice; the replacement reference is not an instruction to reconnect.
- Operable invocation is synchronous and has no deadline, cancellation, or
  asynchronous completion model yet.
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

## Revised recommendation

Keep the lifecycle rule:

```text
subscribe to one configured contract
  -> receive observations
  -> receive terminal notice when that contract changes
  -> deliberately discover and subscribe again
```

Do not preserve a subscription across configurations. The next identity
experiment should test whether a Product can pin a configured Component
Manifest directly and whether named ports are needed only for Components with
multiple genuine Observable interfaces. Separate Output identities should not
move into a production crate until they earn their existence under this
simpler lifecycle.
