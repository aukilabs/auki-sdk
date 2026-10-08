# auki-relay-booking

Book, renew, and release relay capacity through DMS. `AukiPeer` handles this
for your app.

See [connection setup](../../docs/how-to/connect.md) and
[relay settings](../../docs/reference/networking.md#configuration).

`RelayBillingAcceptance::new(policy_version, slot_hour_price, max_credits)` enables
explicit paid acceptance on a create request via `with_billing`. Monetary values
are positive decimal strings (up to six places), never floating-point numbers.
The DMS response repeats the accepted values. Default requests omit `billing`.

The HTTP change is additive. Rust callers that construct request/snapshot structs
with literals must add `billing: None` (or explicit acceptance), and exhaustive
matches on `RelayErrorCode` must handle `PaymentRequired`. Constructor-based free
requests retain their existing behavior.

`RelayErrorCode::PaymentRequired` maps to HTTP 402 only on create/renew. It is not
automatically retryable. An uncertain create must keep its original idempotency
key. A funding denial does not cancel already funded authority; observe the
existing expiry and decide explicitly whether to request more spending later.
See the [SDK paid-relay guide](../auki-sdk/README.md#paid-relay-opt-in-rust-native-and-wasm)
for organization enrollment and the per-booking ceiling semantics.
