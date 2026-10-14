# auki-relay-booking

Book, renew, and release relay capacity through DMS. `AukiPeer` handles this
for your app.

See [connection setup](../../docs/how-to/connect.md) and
[relay settings](../../docs/reference/networking.md#configuration).

Booking requests and responses retain the existing wire format. DMS selects the
server price and funds bookings from the organization's available network
credits when billing is enabled. There are no client billing fields or settings.

`RelayErrorCode::PaymentRequired` maps to HTTP 402 on create/renew. Exhaustive
Rust matches on that enum must handle the new variant; request and snapshot
struct literals need no changes. A 402 is not automatically retryable and must
not trigger credential refresh. An uncertain create keeps its original
idempotency key. A funding denial does not cancel already funded authority;
observe its existing deadline. See the [SDK relay billing guide](../auki-sdk/README.md#relay-billing).

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
The runner also invokes `paid_wss_refresh_expiry_and_persistent_restart` through
the native `AukiPeerBootstrap` / `AukiPeer` facade. It verifies 27 relayed 8-KiB
WSS echoes, issuer/hostname TLS rejection, the actual scheduled credential refresh
and a 401-triggered App service-token exchange, persistent same-Peer-ID restart,
and fail-closed literal expiry during a DDS outage. Refresh keeps the reservation;
an explicit restart creates a new booking. Both ledgers must conserve exact time,
charges and the 80% entitlement, stop at expiry and release every hold.

API/DDS issuance is a local App fixture with one-time bearer-bound Ed25519 proofs.
It ages otherwise valid 30-minute credentials to exercise real timers. DNS and an
ephemeral CA are supplied per test instance; the CA is never installed in the OS.
The P2P `test-support` feature is enabled only by the SDK's native dev-dependency;
the facade transport seam is compiled only for unit tests. Ordinary builds retain
their resolver and WebPKI roots. No certificate or token verification is disabled.

Use `--transport-scenario identity` or `failover` for a focused run; the default
is both. Browser execution, real API/DDS login/issuance, hosted TLS/DNS, and
launch-scale throughput still require separate coverage.
