# Python-Backed Stream Teardown Design

## Problem

An inbound stream request to a Python producer can panic in
`stream_runtime::pump_typed`. On the Booster K1 this repeated until a
Python-backed `futures::stream::unfold` was polled after termination, at which
point the Python/Rust process aborted. Consumers then observed the producer
leave and rejoin the cluster.

Two unsafe teardown edges exist:

1. `pump_typed` treats a closed shutdown watch channel as a non-shutdown update
   and immediately retries. A closed channel remains ready forever.
2. `python_iter_into_source_stream` exposes a bare `Unfold`. `Unfold` panics
   when polled after it has returned `None`.

## Design

### Producer pump

Extract the selection between the source and the shutdown watch channel into a
private `next_pump_event` helper. It returns one of:

- `Source(item)` when the source produces an item or ends.
- `ShutdownRequested` when the runtime explicitly broadcasts shutdown.
- `ShutdownChannelClosed` when the runtime owner disappears without the
  explicit shutdown broadcast.

`pump_typed` preserves the existing wire behavior:

- `ShutdownRequested` writes best-effort
  `EndOfStream(ProducerShuttingDown)`.
- `Source(None)` writes `EndOfStream(SourceEnded)`.
- `Source(Some(Err))` writes `EndOfStream(ProducerError)`.
- `ShutdownChannelClosed` drops the substream without claiming a graceful
  shutdown. Consumers observe the existing `ConnectionLost` behavior.

An ordinary watch update whose value remains `false` is ignored and the helper
continues waiting.

### Python source boundary

Wrap the Python-backed `Unfold` with `StreamExt::fuse()` before erasing it into
`SourceStream<T>`. Once the Python iterator terminates, every subsequent poll
returns `None` instead of panicking.

This is defense in depth: the producer pump should not repoll a terminated
source, but the FFI boundary must remain safe if another SDK caller does.

## Tests

1. A closed shutdown sender produces `ShutdownChannelClosed` promptly.
2. An explicit `true` shutdown broadcast produces `ShutdownRequested`.
3. Repeatedly recreated Python-backed sources can be polled after exhaustion
   and keep returning `None`.
4. Dropping a suspended Python-backed source closes its generator and runs its
   cleanup block.
5. Existing source-ended and producer-shutdown integration tests remain green.
6. Focused Rust and Python binding suites pass under serial Python test
   execution.

## Non-goals

- No Park or AmbientMovement reconnect-policy changes.
- No stream wire-protocol changes.
- No new public API.
- No broad runtime or Python bridge refactor.

## Rollout

After merge, rebuild and reinstall `auki-network-py` and dependent Python
bindings on the K1, restart BoosterApp, then verify repeated Park and
AmbientMovement subscriptions no longer produce panics or peer churn.
