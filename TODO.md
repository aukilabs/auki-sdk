# TODO

## SDK Serve Runtime

Goal: move Sentinel/demo orchestration into `crates/auki-p2p` so app code registers offers and sources while the SDK owns fair inbound serving, active subscription streaming, cancellation, and diagnostics.

### Why

- The current preview Sentinel example manually interleaves lifecycle, offer catalog, Get, Subscribe, frame production, subscription writes, cancellation, and status printing.
- While a stream is active, frame ticks can starve inbound Get/new Subscribe handling.
- Active subscriptions are written sequentially, so one slow consumer can delay other consumers and unrelated control traffic.
- This is too much scheduling responsibility for app developers.

### Non-Goals

- Do not optimize final media transport yet.
- Do not add macOS camera capture in this slice.
- Do not remove the low-level `AukiNode::serve_next_*` APIs.
- Do not put Sentinel-specific business logic in the SDK.
- Do not change RFC frame shapes unless a test exposes a protocol gap.

### Target API Shape

- Add an SDK-owned runtime type, likely `AukiServeRuntime` or `AukiServer`, in `crates/auki-p2p`.
- Constructor takes an `AukiNode`.
- Apps register local domains/offers/providers using existing `AukiNode` APIs or thin runtime forwarding methods.
- Apps register generic Subscribe byte sources in the SDK core.
- Apps register optional generic Get snapshot providers in the SDK core.
- Preview-specific helpers stay as convenience wrappers on top of generic byte sources.
- Runtime exposes:
  - `run_until_shutdown(...)`
  - `tick(...)` or `run(...)`
  - status snapshot/counters
  - shutdown that ends active subscriptions with `producer_shutdown` or `offer_withdrawn`.

### Concurrency Model

- One owner task holds `AukiNode` and polls libp2p/swarm events.
- Inbound protocol serving is fair:
  - lifecycle
  - offer catalog
  - Get
  - Subscribe start
- Get must be served while Subscribe streams are active.
- New Subscribe starts must be accepted while existing subscriptions are active.
- Active subscriptions should not be written by one monolithic app loop.
- Use bounded channels or equivalent per-subscription queues so one slow consumer does not block all consumers.
- Producer frame generation should be independent from network writes.

### Subscription Runtime

- Each accepted subscription gets a runtime record:
  - peer id
  - domain id
  - offer id
  - selected payload type
  - next sequence or source cursor state
  - frame queue/backpressure state
  - cancellation state
- Consumer `SubscribeEnd(cancelled)` removes only that subscription.
- Producer shutdown sends `SubscribeEnd(producer_shutdown)` where possible.
- Offer withdrawal sends `SubscribeEnd(offer_withdrawn)`.
- Normal finite source completion sends `SubscribeEnd(complete)`.

### Backpressure Policy

- Use bounded queues per subscription.
- SDK runtime exposes explicit policies:
  - `LatestOnly`
  - `Bounded { capacity }`
  - `CloseOnFull { capacity }`
- Preview helpers default to `LatestOnly`.
- `LatestOnly` means old queued frames are discarded and the subscriber receives the newest available frame.
- Record dropped frames and slow-consumer closes in status.
- Do not let a slow subscriber block Get, new Subscribe, lifecycle, or other subscribers.
- A slow consumer that must be closed gets `SubscribeEnd(error)` with a structured backpressure/slow-consumer error code.
- Do not use `producer_shutdown` for slow consumers; reserve it for intentional producer/runtime shutdown.

### Status Surface

- Runtime status is typed Rust structs first.
- JSON projection can be added later for bindings or CLI output.
- Runtime status should report:
  - active subscription count
  - active subscriptions by peer/domain/offer
  - frames produced
  - frames sent
  - frames dropped
  - subscriptions cancelled
  - subscriptions closed by producer
  - subscriptions closed for slow consumer/backpressure
  - Get served/rejected
  - Subscribe accepted/rejected
  - last failures
- Preview Sentinel should print these without manually computing them all.

### Tests

- Unit/integration tests in `crates/auki-p2p`:
  - Get is served while a subscription is active.
  - New Subscribe is accepted while another subscription is active.
  - Multiple subscribers receive frames independently.
  - Consumer cancel removes only that subscriber.
  - Slow subscriber does not block a fast subscriber.
  - Runtime shutdown ends active subscriptions.
  - Status counters update for Get, Subscribe, cancel, close, dropped frames.
- Keep browser SDK tests for `SubscribeEnd(cancelled)` compatibility.
- Keep Sentinel example tests focused on CLI/config/generated frame helpers after moving orchestration into SDK.

### Migration Plan

- Phase 1: Design minimal `AukiServeRuntime` API and types.
- Phase 2: Implement runtime-owned fair inbound polling.
- Phase 3: Prove Get is served while one Subscribe is active.
- Phase 4: Add active subscription manager with per-subscription cancellation.
- Phase 5: Prove two subscribers can be active concurrently.
- Phase 6: Add bounded fanout/backpressure for generated preview frames.
- Phase 7: Refactor `examples/p2p-preview-sentinel` onto the SDK runtime.
- Phase 8: Remove now-redundant manual scheduling from the example.
- Phase 9: Re-test browser demo:
  - Get disabled while streaming for now.
  - Then re-enable Get once Sentinel proves Get while streaming is stable.

### Decisions

- SDK core exposes generic byte-source APIs; preview helpers wrap them.
- Preview streams use `LatestOnly` by default.
- Slow consumers close with `SubscribeEnd(error)` and a structured slow-consumer/backpressure error code.
- Runtime status starts as typed Rust structs.
- `AukiServeRuntime` owns polling strategy. Do not add public `try_serve_next_*` APIs unless runtime implementation proves they are needed.
- Build the SDK runtime now instead of only patching Sentinel, because multi-peer streaming is a core SDK behavior.

### First Implementation Slice

- Add minimal `AukiServeRuntime` and status structs.
- Runtime owns one `AukiNode`.
- Runtime can register or reuse existing local Get/Subscribe providers.
- Runtime fairly polls inbound lifecycle, offer catalog, Get, and Subscribe.
- Add a test proving Get is served while a Subscribe is active.
- Keep the implementation single-threaded at first if that is enough to prove fair serving.
- Do not refactor Sentinel until the runtime test proves the starvation fix.
