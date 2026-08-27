# Typed dataflow prototype: first results

Date: 2026-08-27

Branch: `codex/typed-dataflow-prototype`

## Result

The model is viable as an application-facing vocabulary, but this
implementation does **not** justify forcing every local connection or every
remote stream through one runtime path.

The strongest result is the split implementation:

- a static, concrete inline connection can match a handwritten call;
- runtime-connectable named ports are easy to compose but measurably more
  expensive;
- a lightweight Buffer can retain and fan out shared payloads correctly, but
  this prototype's locking and cursor machinery are substantially slower than
  the current `CameraFrameHub`;
- the Buffer-backed pump costs more than the direct pump control in this
  throughput-oriented run, so a mandatory Buffer source for every remote
  stream has not yet earned its cost;
- asynchronous chunks remain a storage experiment. No measured storage or
  replay benefit yet justifies making them the Buffer's retention unit.

The recommendation is to continue with two local connection paths and optimize
the Buffer prototype before deciding whether Buffer-backed remote streaming is
universal.

## Implemented graph

```text
FakeCamera output.frames
  ├─ InlineEvery ─→ MeanBrightness input.frames
  │                   └─ output.level ─→ Level Buffer
  ├─ Latest ─→ SlowPreview input.frames
  └─ Camera Buffer
       ├─ StreamPump ─→ Remote Camera Buffer
       │                 └─ BufferReader ─→ RemoteDetector input.frames
       │                                      └─ Detections Buffer
       ├─ promote shared envelopes ─→ Episode
       └─ asynchronous ChunkBuilder
```

The demo confirms that the source Camera Buffer, remote Buffer, and promoted
Episode can reference the same immutable envelope and payload storage.

## Correctness evidence

Sixteen integration tests and one compile-fail documentation test cover:

- incompatible payload types rejected by the compiler;
- ordered, exactly-once `InlineEvery` delivery;
- ordered `QueuedEvery` delivery with bounded backpressure;
- explicit disconnection rather than silent loss when an `Every` queue fills;
- deterministic `Latest` replacement accounting;
- eight owning consumers sharing one 6 MiB payload and one envelope;
- isolation of a blocked latest consumer from an unrelated queued consumer;
- entry- and byte-bounded Buffer eviction;
- cursor gap reporting and bounded cursor-owned state;
- connection teardown and release of queued payload ownership;
- zero-copy Buffer-to-Episode promotion;
- per-recipient StreamPump cancellation;
- preservation of source sequences and remote gap evidence;
- zero-copy Buffer-to-remote-Buffer delivery;
- live delivery independent of chunk sealing.

## Raw benchmark

This is one unpinned local run. The tiny direct/static measurements are
sensitive to host noise; repeat them before treating small differences as a
decision. The benchmark source is checked in and prints its environment.

- compiler: `rustc 1.94.1 (e408947bf 2026-03-25)`
- target: `aarch64-macos`
- profile: release
- iterations: 20,000,000 for small synchronous cases
- CPU affinity: not pinned

| Case | Iterations | ns/publish | Publishes/s |
|---|---:|---:|---:|
| Direct concrete call | 20,000,000 | 2.23 | 448,601,907 |
| Static `InlineEvery` | 20,000,000 | 2.19 | 456,864,384 |
| Dynamic `InlineEvery` | 20,000,000 | 40.05 | 24,969,372 |
| One-entry Buffer append | 20,000,000 | 78.33 | 12,765,941 |
| Eight Buffer fan-out, one shared 6 MiB handle | 50,000 | 231.18 | 4,325,649 |
| Current `CameraFrameHub`, eight stalled subscribers | 50,000 | 21.77 | 45,925,997 |
| Direct latest pump publish | 100,000 | 121.66 | 8,219,685 |
| One-entry Buffer then pump publish | 100,000 | 181.81 | 5,500,273 |

In that pump run, the direct one-slot pump replaced 88,984 pending values. The
one-entry Buffer cursor skipped 98,905 entries across 3,011 observed gap
events. These paths are intentionally saturated controls; the loss counts show
that their publish-throughput numbers are not an end-to-end quality comparison.

## Interpretation

The static path passed the draft 5% gate in this run. A dedicated
`static_codegen_probe` was also compiled with `--emit=asm`; its AArch64 loop
contained the payload arithmetic and sequence increment directly, with no
function call or indirect branch in the per-payload loop. That supports the
specific monomorphization hypothesis, though it does not make every future
static Component graph automatically zero-cost.

The dynamic path was about 18 times the direct-call cost, although its absolute
cost was about 40 ns per small publication. The cost comes from subscriber-list
locking, reference-counted handle cloning, and indirect dispatch. It should not
be described as compiler-equivalent to the static path.

The one-entry Buffer added about 38 ns over dynamic inline publication. The
current `CameraFrameHub` was about 3.6 times faster than one typed Buffer append
in this particular stalled-consumer publish benchmark. `CameraFrameHub` does
less: it has no retained-range query, promotion, byte budget, or explicit
source-gap model. Even so, it demonstrates that the prototype Buffer's current
mutex/`VecDeque` implementation is not yet competitive.

The eight-Buffer case is deliberately harsher than eight readers of one
Buffer: it represents eight distinct owning data products and locks all eight
rings. Eight Components reading one Buffer use independent sequence cursors and
do not cause eight Buffer insertions.

The Buffer-backed pump's publish call was about 49% slower than the direct
latest-pump control. This is material, but not yet dispositive: the two paths
reported different loss patterns under extreme saturation, and this run did
not measure p50/p99 publish-to-recipient latency. The universal Buffer-source
rule remains an open question.

## Deliberately unresolved

- Allocation counts, copied-byte counts, CPU affinity, p50/p99 latency, and
  peak retained memory still need a proper benchmark harness.
- The current stored-Log detector path was not blended into this local dispatch
  benchmark; its disk and decoding costs need a separate end-to-end test.
- The chunk sidecar proves lifecycle separation only. It does not yet make a
  Buffer and Episode share sealed chunks, and it has not demonstrated a memory,
  replay, encoding, or persistence win.
- `BufferReader` and queued connections currently use one OS thread per
  consumer. That is simple and testable, not a proposed production scheduler.
- Component errors are not modeled. A panic still crosses an inline connection
  and terminates a worker on an asynchronous connection.
- Timestamp order is not enforced. Duration retention assumes the producer's
  timestamps are suitable for window comparison.
- The benchmark does not yet cover GPU/DMA handles or truthful external-memory
  byte accounting.
- The agent-friendliness exercise from the design document has not been run.

These omissions are reasons to keep the code under `experiments/`, not reasons
to smooth over the current results.
