# Native circuit handover experiment

This is an isolated prototype, not a supported SDK API or an automatic expiry
manager. It starts no cloud services and makes no API/DDS/DMS requests. The Go
fixture binds only loopback and mocks relay admission; the Rust peers use the
existing SDK fixture credentials and real mutual authentication. Do not deploy
the Go fixture.

The non-default `auki-p2p/_handover_experiment` feature adds
`AuthenticatedRouteStream::retire_circuit_for_experiment()`. It removes only the
old circuit's reuse mapping, leaving existing owners/streams alive. Ordinary
exact-route opens then dial one replacement, combining concurrent requests.
Old closes/resets remove the route mapping only if it still points to the old
ConnectionId. This is the key lifecycle change needed to preserve a replacement.

The caller must retain and eventually close every old stream. Repeatedly
retiring successive generations without closing old ones is **not bounded by
this experiment API**. It is deliberately not exposed in the SDK facade or
Python/Swift/Web/Expo bindings. The browser has no retirement API.

## Run

Use an existing Hagall checkout with its vendored `go-libp2p v0.41.1` dependencies.
The fixture is compiled with that checkout's module/vendor selection; the short
circuit lifetime bypasses only Hagall's configuration layer in this local test.
No dependency update or change to Hagall's deployed minimum lifetime is needed.

```sh
# Set these to local checkout paths.
SDK_CHECKOUT=/tmp/auki-sdk-circuit-handover
HAGALL_CHECKOUT=/path/to/hagall

cd "$HAGALL_CHECKOUT"
go build -mod=vendor -o /tmp/auki-handover-go-relay \
  "$SDK_CHECKOUT/test-support/circuit-handover/go-relay/main.go"
go version -m /tmp/auki-handover-go-relay

cd "$SDK_CHECKOUT"
AUKI_HANDOVER_GO_RELAY=/tmp/auki-handover-go-relay \
  cargo test --locked --offline -p auki-p2p --features _handover_experiment \
  --test relay_transport handover_experiment -- --ignored --test-threads=1 --nocapture

cargo test --locked --offline -p auki-p2p -p auki-sdk \
  --features auki-p2p/_handover_experiment
```

Go integration tests are explicitly ignored by default because they require the
fixture binary. They use the existing loopback DNS helper; advertised test names
resolve to 127.0.0.1 without contacting public DNS. Each test owns a relay process
and terminates/reaps it on drop, including assertion failures.

## What the ten integration checks cover

- Forced expiry with a six-second Go circuit limit during eight seconds of traffic.
- Three planned drain-close-reopen rotations before that deadline.
- Three overlapping replacements, with frame-count/last-sequence drain receipts.
- Rejected replacement dial leaves the old stream usable.
- Replacement authentication fails on a valid target credential for the wrong
  Domain; the candidate closes and the old stream remains usable.
- Cancelling an in-flight delayed dial cleans up the eventual candidate circuit.
- Concurrent opens share the new generation; old sibling streams survive.
- Old circuit expiry does not invalidate the new circuit's reuse mapping.
- The old stream sends throughout a deliberately delayed replacement dial.
- Awaited shutdown releases both live generations.

The stream payload is 3,200 bytes at a nominal 10 ms interval. These short debug
tests are correctness experiments, not a repeat of the 256 kbit/s dev load test.
Receivers validate payload, increasing sequence numbers and final per-stream
sequence/count receipts. Go counters verify circuit overlap, closed circuits,
and one reservation. Source admission is reused during each eight-second run.

`HANDOVER_RESULT` records include sent/received counts, missing successful writes,
write failures, drain acknowledgements and Go relay counters. Rotation wall time
includes fixture/control checks and pauses for setup; do not present it as a
production interruption benchmark or evidence that overlap is faster. The
separate delayed-dial test proves old traffic can continue during preparation.

## Recorded local result — 2026-09-23

Built with Go 1.25.5 and Hagall's vendored go-libp2p v0.41.1. Each traffic case
ran for eight seconds; planned cases rotated three times before the six-second
circuit deadline. Counts are complete application payload frames, not packets.

| Case | Successfully written | Received | Missing successful writes | Peak circuits | Reservations |
| --- | ---: | ---: | ---: | ---: | ---: |
| Forced expiry | 791 | 790 | 1 | 1 | 1 |
| Drain-close-reopen | 777 | 777 | 0 | 1 | 1 |
| Overlapping replacement | 774 | 774 | 0 | 2 | 1 |

All cases finished with zero active circuits and matching opened/closed counts.
The planned cases each received four valid drain acknowledgements. Their sent
counts differ because the correctness fixture pauses during rotation and skips
missed send ticks; it does not measure a throughput advantage for overlap.

All ten Go-fixture tests passed, as did 290 native tests across `auki-p2p` and
`auki-sdk` with the experimental feature enabled (the ten explicit Go tests were
run separately). Clippy with warnings denied, formatting, and WASM compilation
of both crates passed. WASM compilation is not browser handover validation.
No binding surface changed, and Python/Swift/Web/Expo runtime suites were not run.

Logs, comparison JSON and Go build provenance were saved in the performance repo
under `relay-load/results/20260923-circuit-handover-local/` (ignored artifacts).
No fixture processes remained afterward. No EC2 instance was started and no
shared dev state was changed.

## Remaining production design

- Atomic prepare/commit/abort replacement ownership, a strict cap on retained
  generations and cleanup when callers cancel or forget old handles.
- Conservative circuit age/deadline tracking, correctly correlated advertised
  limits, data-cap handling, bounded retries and jitter. Do not use credential
  expiry as the circuit deadline.
- An explicit application handover boundary. Pre-open/authenticate, finish and
  acknowledge old payload, then send new payload. Opaque existing streams cannot
  be migrated transparently; arbitrary two-stream sending can reorder frames.
- Browser/WSS runtime coverage and an SDK facade/binding design before release.
- Full Hagall admission/provider integration and a controlled repeat of the dev
  workload. This fixture has no DMS, so it cannot independently prove DMS call
  counts; it establishes that replacement works on one relay reservation.
- Acknowledgement/replay/deduplication if delivery must survive unplanned failures.
  Successful local planned handovers do not establish a general zero-loss guarantee.
