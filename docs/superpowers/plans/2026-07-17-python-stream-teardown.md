# Python-Backed Stream Teardown Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prevent producer teardown and exhausted Python-backed streams from panicking or hot-looping.

**Architecture:** Isolate producer pump selection in a private helper that distinguishes explicit shutdown from a dropped shutdown owner. Fuse the Python-backed stream before type erasure so termination remains stable under repeated polling.

**Tech Stack:** Rust, Tokio watch channels and `select!`, futures `Stream`/`Fuse`, PyO3, cargo test.

## Global Constraints

- Preserve the current stream wire protocol and public API.
- Keep explicit producer shutdown mapped to `EndReason::ProducerShuttingDown`.
- Treat a dropped shutdown owner as an unclean transport end.
- Follow red-green TDD for each behavior.
- Do not commit unless the developer explicitly requests a commit.

---

### Task 1: Make Producer Pump Teardown Explicit

**Files:**
- Modify: `crates/auki-network/src/stream_runtime.rs:479-526`
- Test: `crates/auki-network/src/stream_runtime.rs:613-646`

**Interfaces:**
- Produces: private `PumpEvent<T>` enum.
- Produces: private `next_pump_event<T>(&mut SourceStream<T>, &mut watch::Receiver<bool>) -> PumpEvent<T>`.
- Preserves: `pump_typed<T>` and all public stream APIs.

- [x] **Step 1: Write the failing closed-owner test**

Add a unit test with a pending camera source, drop the `watch::Sender`, and
assert the helper resolves promptly:

```rust
#[tokio::test]
async fn closed_shutdown_channel_ends_the_pump_wait() {
    let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
    drop(shutdown_tx);
    let mut source: SourceStream<CameraFrame> = Box::pin(stream::pending());

    let event = tokio::time::timeout(
        Duration::from_millis(100),
        next_pump_event(&mut source, &mut shutdown_rx),
    )
    .await
    .expect("closed shutdown channel must not hot-loop");

    assert!(matches!(event, PumpEvent::ShutdownChannelClosed));
}
```

- [x] **Step 2: Run the test and verify red**

Run:

```bash
cargo test -p auki-network stream_runtime::tests::closed_shutdown_channel_ends_the_pump_wait -- --exact
```

Expected: compile failure because `next_pump_event` and `PumpEvent` do not yet
exist.

- [x] **Step 3: Add explicit pump events and selection**

Add:

```rust
enum PumpEvent<T> {
    Source(Option<Result<StreamItem<T>, String>>),
    ShutdownRequested,
    ShutdownChannelClosed,
}

async fn next_pump_event<T>(
    source: &mut SourceStream<T>,
    shutdown_rx: &mut watch::Receiver<bool>,
) -> PumpEvent<T>
where
    T: Send + 'static,
{
    loop {
        tokio::select! {
            biased;
            result = shutdown_rx.changed() => match result {
                Ok(()) if *shutdown_rx.borrow() => {
                    return PumpEvent::ShutdownRequested;
                }
                Ok(()) => continue,
                Err(_) => return PumpEvent::ShutdownChannelClosed,
            },
            item = source.next() => return PumpEvent::Source(item),
        }
    }
}
```

Replace the inline `tokio::select!` in `pump_typed` with a `match` over
`next_pump_event`. Return immediately for `ShutdownChannelClosed`, preserve
typed end reasons for every other branch.

- [x] **Step 4: Verify green and existing shutdown behavior**

Run:

```bash
cargo test -p auki-network stream_runtime::tests::closed_shutdown_channel_ends_the_pump_wait -- --exact
cargo test -p auki-network stream_runtime::tests::producer_shutdown_signals_consumer_with_typed_end_of_stream -- --exact
cargo test -p auki-network stream_runtime::tests::producer_accepts_and_streams_camera_frames -- --exact
```

Expected: all three pass.

### Task 2: Fuse Python-Backed Sources

**Files:**
- Modify: `bindings/python/auki-network-py/src/stream_types.rs:1668-1729`
- Test: `bindings/python/auki-network-py/src/stream_types.rs:2182-2194`

**Interfaces:**
- Consumes: existing `python_iter_into_source_stream<T>`.
- Preserves: its `SourceStream<T>` return type and Python API.

- [x] **Step 1: Write the repeated-exhaustion regression test**

Create an empty Python async generator, convert it to a source stream, and poll
it twice:

```rust
#[test]
fn exhausted_python_source_remains_terminated() {
    pyo3::prepare_freethreaded_python();
    let _ = crate::stream_bridge::asyncio_locals();
    let aiter = Python::with_gil(|py| {
        let module = PyModule::from_code_bound(
            py,
            "async def _gen():\n    if False:\n        yield None\n",
            "test_empty_source.py",
            "test_empty_source",
        )?;
        Ok::<Py<PyAny>, PyErr>(module.getattr("_gen")?.call0()?.unbind())
    })
    .unwrap();
    let mut source = python_iter_into_source_stream::<RustCameraFrame>(
        aiter,
        |_| unreachable!("empty source must not invoke conversion"),
    );

    crate::cluster_tokio_runtime().block_on(async {
        assert!(source.next().await.is_none());
        assert!(source.next().await.is_none());
    });
}
```

- [x] **Step 2: Run the test and verify red**

Run:

```bash
cargo test -p auki-network-py stream_types::tests::exhausted_python_source_remains_terminated -- --exact --test-threads=1
```

Expected: panic on the second poll because bare `Unfold` is not fused.

- [x] **Step 3: Fuse before type erasure**

Change:

```rust
Box::pin(stream)
```

to:

```rust
Box::pin(stream.fuse())
```

- [x] **Step 4: Verify green**

Run the same exact test. Expected: both polls return `None` and the test passes.

### Task 3: Focused Verification

**Files:**
- Verify: `crates/auki-network/src/stream_runtime.rs`
- Verify: `bindings/python/auki-network-py/src/stream_types.rs`

**Interfaces:**
- No new interfaces.

- [x] **Step 1: Format-check edited Rust**

```bash
cargo fmt --all -- --check
```

- [x] **Step 2: Run focused suites**

```bash
cargo test -p auki-network stream_runtime::tests::
cargo test -p auki-network-py --lib stream_types::tests:: -- --test-threads=1
cargo test -p auki-network-py --lib stream_bridge::tests:: -- --test-threads=1
```

- [x] **Step 3: Run focused lint**

```bash
cargo clippy -p auki-network -p auki-network-py --all-targets -- -D warnings
```

- [x] **Step 4: Inspect the final diff**

Confirm only the two implementation files, their in-file tests, and the issue
319 design/plan documents changed.

## Execution Notes

- Both RED states were observed: the missing producer-pump helper failed to
  compile, and the bare Python `Unfold` panicked on its second terminal poll.
- The three critical network tests pass individually with `--features swarm`.
  The full module remains connect-flaky and produced unrelated connection
  timeouts.
- Python stream tests pass: 31 `stream_types` tests and 3 `stream_bridge`
  tests. Coverage includes repeated source recreation and Drop-triggered
  generator cleanup.
- Both edited Rust files pass targeted `rustfmt`; `git diff --check` and IDE
  diagnostics are clean.
- Workspace-wide formatting and strict Clippy remain blocked by pre-existing
  drift and errors in unchanged files and dependencies.
