# Typed dataflow complete network-independent prototype

Date: 2026-09-02

Branch: `codex/typed-dataflow-complete`

## Outcome

The network-independent public shape is now implementable without copying the
Camera fixture or independently fabricating Catalog entries.

The second-pass construction invariant is:

```text
PeerRuntime
  -> Component
       -> ConfiguredObservable<T> / Operable<I, R>
       -> expose only when every declared interface is live
            -> read-only Catalog projection

ConfiguredObservable<T>
  -> direct or serialized observers
  -> Buffer Product
  -> Episode Product
```

Applications cannot mutate the Catalog directly. A Component cannot be
exposed with a missing or already-dropped interface. A fresh Observable cannot
advertise retained access that it cannot answer. `ContractType` binds its Rust
payload, instruction, and result types to advertised datatypes, while the
modality-specific payload contract describes schema and semantics explicitly.

This is evidence for the vocabulary and lifecycle, not production SDK code.
No production networking, Manager, heartbeat, Domain, Registry, Log, or
streaming implementation was changed.

## Public clean-room application

`auki-typed-dataflow-volume-monitor` is a separate workspace crate which uses
only public APIs. It constructs two Peers, each with:

```text
Microphone.audio
  |- 6,000-block / 60-second Buffer Product
  `- Volume Meter -> level
                      `- session Episode Product

Peer A level -> serialized in-memory boundary -> Peer B observer
Peer B level -> serialized in-memory boundary -> Peer A observer
```

The audio contract exposes `f32`, interleaved layout, 48 kHz, one channel, and
480 frames per 10 ms block. The derived Gauge exposes `audio_level`,
`float64`, and `dBFS`; silence uses a finite -120 dBFS floor.

Four application integration tests and one compile-fail test prove:

- typed local Microphone-to-Volume composition;
- 6,000-entry Buffer eviction;
- whole-session Episode retention and one explicit conclusion;
- shared local audio allocation across publication, Buffer, and Volume input;
- a distinct allocation and counted bytes across serialization;
- remote handle drop stopping subsequent delivery;
- two Component and two Product Catalog entries per Peer;
- typed audio and Gauge contract fields in the Catalog.

The application is 274 nonblank, non-comment source lines across its library
and executable. This count includes error plumbing and the reusable `VolumePeer`
fixture. It is not a blind agent-friendliness score: the same implementation
round designed the construction API. An unfamiliar colleague or new agent
still needs to run the preserved task from public documentation only.

## Correctness added in this round

The prototype test suite now also covers:

- Observable-only, Operable-only, and combined Components;
- exposure rejected for missing, dropped, mismatched, or falsely capable
  interfaces;
- compile-time Observable and Operable type mismatch;
- equivalent local and serialized live delivery for ordered-every and
  coalescing-latest policies;
- caller Peer and caller Component identity at the target handler;
- synchronous local and serialized Operable result equivalence;
- serial-in-acceptance-order Operables;
- inspectable asynchronous success, rejection, panic, cancellation, deadline,
  and overload outcomes;
- a hard per-Operable outstanding-invocation limit;
- cancelled queued instruction ownership released after scheduler drain;
- strict, non-decreasing, and unordered source timestamp policies;
- explicit source-time versus arrival-time duration eviction;
- compile-time rejection of a missing source timestamp;
- external allocation leases surviving Buffer eviction;
- truthful external-byte accounting against a hard Buffer limit;
- local external-storage fan-out without readback;
- one explicit, counted external-to-serialized readback and a distinct remote
  allocation.

## Retained storage and chunks

The tests now answer the important ownership question without elevating chunks
into the model:

- Buffer and Episode share the exact same immutable `Arc<Envelope<T>>`;
- Buffer eviction releases only the Buffer's lease;
- the Episode continues retaining the allocation;
- the external allocation is released after the last independent lease;
- promotion works while the asynchronous ChunkBuilder still has an open,
  unsealed partial chunk;
- live pumping remains independent of chunk sealing.

The ChunkBuilder has demonstrated lifecycle separation but no replay,
encoding, persistence, memory, or promotion win. The recommendation is to
remove chunks from the next public design unless a concrete storage backend
later supplies such a win. “Sealed chunk” should not be a correctness
requirement for Buffer-to-Episode promotion.

## Recorded benchmark controls

These are unpinned local measurements, not universal performance claims.

- compiler: `rustc 1.94.1 (e408947bf 2026-03-25)`
- target: `aarch64-macos`
- profile: release
- CPU affinity: unavailable / not pinned

### Publication paths

One 500,000-iteration run:

| Case | ns/publication |
|---|---:|
| Handwritten direct call | 2.38 |
| Static `InlineEvery` | 2.22 |
| Dynamic `InlineEvery` | 40.01 |
| One-entry Buffer append | 88.21 |
| Eight Buffer fan-out, shared 6 MiB handle | 378.93 |
| Direct latest pump | 120.21 |
| One-entry Buffer then pump | 212.58 |

The process used 0.12 seconds user CPU and 0.06 seconds system CPU, with
27,541,504 bytes maximum resident set size. Saturated pump controls reported
92,846 replacements on the direct latest path and 98,442 skipped source
entries in 4,564 Buffer cursor gap events.

### Configured Observable paths

Seven samples after warmup, median of 100,000 publications per sample except
the 20,000-publication serialized path:

| Case | Median ns/publication |
|---|---:|
| Camera, no observer | 407.76 |
| One local subscription | 434.61 |
| Two local subscriptions | 454.57 |
| Serialized in-memory observer | 1,795.93 |

The serialized samples encoded and decoded 150,000 messages / 63,506,670 bytes
in each direction. The marginal second local observer was 19.96 ns.

### Operable paths

Seven samples after warmup:

| Case | Median ns/invocation |
|---|---:|
| Handwritten typed operation | 0.38 |
| Dynamic local `Operable` | 132.42 |
| Serialized in-memory `Operable` | 611.64 |
| Shared-scheduler async `Operable` | 9,139.36 |

The async path measured p50 8,167 ns and p99 30,709 ns end-to-end latency.
Serialized samples encoded and decoded 290,000 messages / 22,421,128 bytes in
each direction.

### Scheduler control

The existing one, eight, 64, and 256 relationship comparison was rerun with
three samples, 200 observations per relationship, and four shared workers.
At 256 relationships, median publication cost was 741,285 ns with one thread
per relationship and 23,440 ns with four shared workers. Completion latency
was 3,083 ns versus 7,217,583 ns respectively because the per-thread path had
already drained during its much slower publication loop. Both models isolated
the deliberately blocked observer. The result continues to support scheduler
choice as private runtime policy, not Observable semantics.

## Measurement limitations

The harness counts semantic payload allocations, retained payload bytes,
serialized bytes, replacements, overruns, gaps, and task/thread construction.
It does not yet count every allocator call made inside `Arc`, locks, queues,
or JSON. The crate forbids unsafe code and does not install a process-wide
counting allocator. Use an external allocation profiler before making a
production implementation choice.

The current stored-Log detector comparison was deliberately not run in this
round because production SDK integration was excluded. Cold-cache disk
measurements also require a separate controlled harness.

## Remaining external validation

Only two activities remain outside this network-independent implementation:

1. a genuinely blind agent/developer run of the preserved clean-room prompt;
2. production SDK and stored-Log integration/comparison, when explicitly
   authorized.

Separate Output identity and named output slots remain provisional. The
prototype proves their current behavior but does not yet prove that both are
necessary in the final public model.

## Verification

The final revision passed:

- 57 prototype integration tests;
- four external volume-monitor integration tests;
- four prototype compile-fail documentation tests;
- one external Observable compile-fail documentation test;
- the typed-dataflow, Observable/Operable, and volume-monitor demos;
- focused Clippy for both crates with warnings denied;
- package-scoped formatting checks.
