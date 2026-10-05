# Native inbound stream investigation — 2026-09-22

Base SDK revision: `1007d515cd6af3c3d2473c23d2c1651c52de21a2`.
Branch: `fix/native-inbound-stream-bursts`.

## Findings

Two native SDK problems contributed to stream-open failures:

1. `Node::serve` selected handler completion against `incoming.accept()`, which
   owned an unfinished mutual-authentication handshake. Finishing one handler
   could drop another peer's handshake. A controlled localhost test reproduced
   `UnexpectedEof` in five consecutive runs; keeping the old handler alive passed.
2. Native acceptance authenticated one stream at a time. The stock
   `libp2p-stream` inbound handoff uses `mpsc::channel(0)` and `try_send`, so a
   burst can overflow it before the receiving task is scheduled. A paused
   handshake plus nine other valid peers reproduced `WriteZero: connection is
   closed`. Concurrent authentication alone still reproduced burst drops.

A cancellation-only candidate passed its local regression but failed a 60-second
optimized dev reconnect run: 36 successful opens, 11 failed opens, and 16,352,000
payload bytes both sent and received. Temporary phase diagnostics in a separate
checkout then identified every failure in the next run as `WriteZero` during
peer authentication. Relay source-admission rejection counters were unchanged.
These findings also explain why cancellation alone could not account for the
initial-open failure seen in the original load-test baseline.

## Final implementation

- Native managed servers dequeue streams promptly and put authentication and
  application handling in the same supervised task. Both phases share the
  existing protocol concurrency limit. Completing one task cannot cancel
  another handshake, and a slow handshake does not block all other peers.
- A native adapter intercepts negotiated inbound streams before the stock
  zero-buffer handoff. Each protocol has a bounded queue of **64** streams;
  overflow drops the excess stream. Protocol negotiation, duplicate registration,
  outbound behavior, and connection lifecycle remain delegated to libp2p-stream.
- Closing a protocol closes and drains its queue. Managed shutdown cancels and
  awaits handshakes and handlers. No extra detached workers were introduced.
- DDS signature/peer/Domain/expiry checks, wire formats, and public APIs are
  unchanged. Browser behavior is unchanged.

The queue is a transient per-protocol handshake buffer, separate from relay
booking capacity. The tested topology has nine publishers per consumer.

## Local validation

```sh
cargo test --locked -p auki-p2p -p auki-sdk
cargo clippy --locked -p auki-p2p -p auki-sdk --all-targets -- -D warnings
cargo check --locked -p auki-p2p -p auki-sdk --target wasm32-unknown-unknown
cargo fmt --all -- --check
git diff --check
```

The P2P and SDK suites pass **287 tests**. Lint and the WASM compile check pass.
New localhost tests cover handler completion during authentication, its control,
shutdown during authentication, the concurrency boundary, progress of nine peers
while another handshake is paused, queue overflow with 65 streams on one TCP
connection, and closing queued streams followed by remounting the protocol.

The early queue test attempted many independent TCP dials and hit unrelated
connection/OS descriptor limits. The final queue tests isolate the handoff using
65 negotiated streams on one connection; they need no raised OS limits.

Browser execution and language-binding end-to-end tests were not run. The changed
runtime and adapter are native-only; the shared facade compiles for WASM.

## Dev validation setup

All runs use the existing dev Hagall revision
`355bc06d6e5937b1473f533c1af911d1315917a7`, the approved account and Domain, and
256 kbit/s per publisher. No server, infrastructure, quota, or Domain data changes
were made. Reports are under `performance/relay-load/results/`; each patched run
records source hashes. Relay metrics are captured around each run through a
read-only temporary port-forward. Every run verifies booking deletion.

The optimized harness is built in a temporary directory with a Cargo path patch
for `auki-p2p`. An experimental environment variable spaces the first reconnect
by publisher index for the staggered comparison; the chosen value is recorded
in its report. The normal harness source is not changed for these diagnostics.

These are small correctness checks, not evidence of 1,000-peer capacity,
60-minute stability, natural circuit expiry, browser/WSS behavior, or final
load-generator sizing.

## Final dev results

Each run held its steady phase for 60 seconds. Reconnect cases used a 15-second interval.

| Scenario | Peers | Opens | Errors | Delivery per publisher | Report |
| --- | ---: | ---: | ---: | --- | --- |
| Synchronized reconnect 1 | 10 | 36 | 0 | 97.00–97.50% | `20260922T052607Z-7b93bbe0-58c7-4c9d-905c-74d767738faf` |
| Synchronized reconnect 2 | 10 | 36 | 0 | 96.83–97.83% | `20260922T052801Z-67d58275-5595-4c83-bb8d-71295aa4c398` |
| Steady streaming | 10 | 9 | 0 | 100.00–100.00% | `20260922T052917Z-1ec0ed54-8eaf-4234-8a55-0674ed782847` |
| One reconnecting publisher | 2 | 4 | 0 | 97.17–97.17% | `20260922T053033Z-749b4959-12fe-41ae-bf2c-591284e47f19` |
| Staggered reconnects (1 s spacing) | 10 | 36 | 0 | 97.00–97.83% | `20260922T053139Z-9cfcac6d-8941-47bb-8641-fbc082a43bd5` |

All five runs passed their original per-publisher 95% gate, delivered every sent
byte, and confirmed deletion of every test booking. No initial-open retries were
needed. Relay admission/rejection counters did not increase in the first four
runs; the final metric and health check is recorded with the harness validation.
The intentional reconnect pauses account for delivery below the uninterrupted
target; the steady run delivered 100%.
