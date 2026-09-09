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
