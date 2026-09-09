# auki-dms

DMS task types, HTTP operations and a small polling helper, usable without a
Posemesh runner. Native Rust client support is enabled by the default `client`
feature. Use `default-features = false` for `TaskSpec`, `LeaseEnvelope`,
heartbeat/complete/fail DTOs and `DmsPaths` without auth, HTTP or Tokio dependencies.

`DmsClient::new(base, timeout, token_provider)` only constructs a client. The
caller explicitly invokes lease, heartbeat, complete and fail, or supplies a
callback to `poller::run_poller`. The client accepts the existing
`auki_auth::machine::token_manager::TokenProvider` seam.

The host owns task execution, heartbeat scheduling, progress/events, storage,
cancellation policy and completion reporting. Posemesh's compute-node remains
the ready-made runner when those pieces are wanted together.

## Preserved behavior

These blocks are extracted from the existing Posemesh implementation:

- Each HTTP operation replays once after a 401 and token refresh notification.
- Lease responses 204 and 409 both return `None`.
- `lease_by_capability` currently ignores its capability argument. `DmsPaths`
  can build a capability query separately.
- Client URL paths append segments to the supplied base; `DmsPaths` uses URL
  joining. Their existing handling of base paths/trailing slashes is preserved.
- The poller invokes the callback before waiting and observes shutdown during
  its backoff wait, not during an active callback. It uses the existing
  millisecond-derived jitter, which is not uniform across large delay ranges.

No automatic task claim happens during authentication or peer startup.

## Standalone Rust example

[poll_noop.rs](examples/poll_noop.rs) constructs a client from a caller-provided
`DMS_MACHINE_TOKEN` and `DMS_BASE_URL`, then runs a finite loop controlled by
`DMS_POLL_COUNT` (default 1). It uses the existing `TokenProvider` interface and
SDK poller delay helper without implementing any Posemesh runner traits.

Run `cargo run -p auki-dms --example poll_noop` only against a queue dedicated to
the example: it claims and completes `/example/noop/v1` tasks, and reports other
capabilities as failed. The supplied bearer has no automatic refresh; replace
that provider with a machine `TokenManager` for long-lived use. Long tasks need
host-driven heartbeat and cancellation handling.

For a callback loop, explicitly call `poller::run_poller(config, shutdown_rx,
on_tick)`; retain the transmitter returned by `poller::shutdown_channel()` and
call `shutdown()` when the host wants it to stop. The callback owns each request
and its error handling. Auth, client and peer construction never call this helper.

Posemesh preserves `dms::{client,types,DmsPaths}`, `poller`, and runner-api's
`TaskSpec`/`LeaseEnvelope` through re-exports. Existing Runner and storage APIs
remain in Posemesh; runner-api uses this crate without default features.
