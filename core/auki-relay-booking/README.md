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

The companion NCS runner exercises the real SDK coordinator and native transport
against real DMS/NCS and Hagall. From the NCS checkout:

```sh
python3 scripts/test_relay_billing_e2e.py --dms-repo ../domain-manager-service \
  --sdk-repo ../auki-sdk --hagall-repo ../hagall
```

The ignored `paid_booking_renews_and_recovers_real_provider_loss` coordinator test
uses local identity/DNS fixtures and production signature checks. It sends 58
relayed 8-KiB round trips, survives a 45-second accounting outage, waits the actual
provider recovery deadline (up to five minutes, capped by booking authority), renews funded authority and confirms the
replacement route keeps the same booking/slot. Cancellation removes routes and
releases the credit hold. Add `--evm --safe-service` on Docker Desktop for macOS to
follow both providers' earnings through a real two-owner Safe and token claim.
This combined test covers native TCP and the coordinator; the public facade's
identity supervisor, browser WSS ingress and capacity soak need separate coverage.
