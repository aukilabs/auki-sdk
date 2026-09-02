# Volume-monitor clean-room test: first-attempt result

Date: 2026-09-01

Branch: `codex/typed-dataflow-decision-review`

## Result

The public-API feasibility gate failed before the first live Component could be
created.

An external application can publicly construct and register a
`ComponentManifest` and `OutputManifest`. The Catalog accepts both. The
application cannot then publicly create the generic `Observable<AudioBlock>`
and publisher that those Manifests claim exists.

This is worse than ordinary missing convenience. It permits the exact failure
the design is intended to prevent:

```text
Catalog assertion exists
        |
        v
no corresponding live typed producer is required
```

The first attempt stopped rather than fabricating the producer, using private
Camera helpers, or weakening the task.

## Method and limitation

The feasibility probe was an external temporary Rust crate depending only on
the public `auki-typed-dataflow-experiment` API. It:

1. defined a typed `AudioBlock`;
2. created a truthful-looking Microphone Component Manifest;
3. registered it in a new Peer's Catalog;
4. created and registered an audio Output Manifest;
5. attempted to create the live `Observable<AudioBlock>` governed by that
   Output Manifest.

The compiler rejected the last operation:

```text
error[E0599]: no function or associated item named `new` found for struct
`Observable<AudioBlock>` in the current scope
```

The probe did not modify the SDK.

This was not a valid blind agent-friendliness score: the same conversation that
designed the experiment performed the feasibility audit. A genuinely fresh
agent or developer must run the full task after the construction API exists.

## Blockers

### 1. No public generic Component construction path

`CameraComponent` is a useful fixture, but there is no public generic
`Component` handle or builder that binds behavior, interfaces, and a Component
Manifest.

Desired operation:

```text
create Microphone Component
  -> declare typed audio Output
  -> receive a publisher and Observable
  -> expose the live Component
```

Closest public API: manually create `ComponentManifest` and call
`Catalog::register_component`.

Why that is insufficient: it registers a description without constructing or
linking any behavior.

### 2. No public generic configured Output construction path

`Observable<T>` can be inspected and observed but cannot be constructed by an
external application. `ObservationEmitter<T>` and the helpers used by the
Camera fixture are crate-private.

Closest public API: manually register an `OutputManifest`, then use an unrelated
raw `OutputPort<T>`.

Rejected workaround: pairing them by naming convention would let the Manifest
and actual payload pipeline contradict each other.

### 3. Catalog exposure is not coupled to a live interface

The Catalog exposes public low-level mutation methods. It verifies that an
Output references a registered Component Manifest hash, but not that the
Component or Output has a corresponding runtime interface.

The current API therefore enforces referential well-formedness while still
allowing operational nonsense.

### 4. Product construction is fixture-specific or manual

A Buffer can be created, but creating a `RetainedProduct<T>` requires manual
Manifest assembly. Camera Buffer capture is implemented only by the
fixture-specific `CameraBufferCapture`. `Episode<T>` has no corresponding
typed Product wrapper or public Product lifecycle integration.

The volume task cannot truthfully expose its session Episode through the
Catalog without inventing application-specific glue.

### 5. `PayloadContract` is biased toward Camera fields

The common contract contains `width` and `height` but no structured mechanism
for configured audio sample rate, channel layout, or sample representation.
Those facts could be hidden inside a schema name or overloaded encoding string,
but they would not be comparably discoverable typed fields.

This does not prove that every modality needs fields in one closed enum. It
does prove that the present supposedly common shape is not common.

## First-attempt score

The rubric is intentionally not fully scored because no Component could be
created and the test was not blind. The hard-failure gate is enough:

- Component construction: **0/3**;
- Catalog truthfulness by construction: **0/3**;
- Product construction: **0/3**;
- complete application: **blocked**.

The correct disposition for “public API agent-friendliness” remains **Reject
pending a second-pass construction API and a genuinely clean rerun**.

## Smallest proposed second pass

Add one public construction path that creates runtime behavior and its
descriptions together:

```rust,ignore
let microphone = peer.component(ComponentSpec::new("microphone"))?;

let audio = microphone.observable::<AudioBlock>(ObservableSpec {
    contract: audio_contract,
    clock_id: session_clock,
    spatial_frame_id: None,
})?;

audio.publish(timestamp_ns, block)?;
```

The important property is not the exact spelling. It is the invariant:

> An exposed Catalog entry is obtained from a live typed interface; an
> application does not independently manufacture the two and promise they
> correspond.

The same second pass needs generic helpers that derive Buffer and Episode
Product Manifests from an exact Output reference rather than requiring manual
parallel assembly.

Do not implement this change until the decision review confirms:

1. whether Catalog low-level mutation should become crate-private or remain an
   explicitly unsafe/advanced escape hatch;
2. whether `Component` should be a runtime handle, trait, builder result, or a
   smaller combination;
3. how modality-specific configured contract fields remain standardized and
   extensible without returning to a loosely typed property bag.
