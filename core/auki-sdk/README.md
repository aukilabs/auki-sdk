# auki-sdk

Sign in, connect to peers, and exchange data inside your app. The SDK also
provides Domain data clients and a managed runtime for compute and robot tasks.

| Task | Start with |
| --- | --- |
| Connect peers with `AukiPeer` | [Connect two peers](../../docs/tutorials/first-peer.md) |
| Resolve a Peer, Robot or Compute identity | [Identity resolution](../../docs/reference/peer-resolution.md) |
| Read and write with `AukiDomains` and `AukiDomainData` | [Work with Domain data](../../docs/how-to/domain-data.md) |
| Inspect inventory with `AukiFleet` | [Inspect a fleet](../../docs/how-to/inspect-fleet.md) |
| Submit and monitor with `AukiDmsJobs` | [Submit Domain jobs](../../docs/how-to/submit-jobs.md) |
| Execute handlers with `AukiDmsTasks` | [Run compute and robot tasks](../../docs/how-to/run-compute-tasks.md) |

For APIs, defaults, and limits, see the [networking](../../docs/reference/networking.md),
[Domain data](../../docs/reference/domain-data.md), and
[task](../../docs/reference/tasks.md) references.

## Planned relay circuit replacement (Rust)

`protocols.prepare_replacement(&old_stream, protocol_id)` prepares and mutually
authenticates a stream on a fresh circuit while the old stream remains usable.
The returned future does not borrow the old stream. Await/poll preparation while
continuing traffic, then use an application-level drain acknowledgement or
checkpoint before sending on the replacement and closing the old stream.

Concurrent preparations from the same old generation share one replacement hop.
The SDK retains at most two circuit generations per exact route. Close all old
sibling streams before replacing the new generation. Failed or cancelled
preparation releases its candidate ownership and leaves old owners alive; if
no candidate owners remain, subsequent opens can reuse the old circuit.
Peer shutdown still releases both generations. Authentication and exact-route
checks apply to every new stream, including replacements.

This API does not schedule expiry, replay bytes, or migrate an opaque stream.
Choose a conservative preparation age from the provider's circuit duration and
data limits, allowing for dial/authentication/drain time and setup age. Booking
or credential expiry is not a circuit deadline. Failures still need application
acknowledgement/replay/deduplication if delivery must be guaranteed. Replacement
uses the current reservation and ordinary relay admission; it does not book a
new relay through DMS.

Native Rust callers can select WSS reservations using
`AukiPeerConfig::with_relay_transport(auki_p2p::RelayBaseTransport::Wss)`.
TCP remains the default. Explicit outgoing relay routes must use the selected
transport. Both addresses remain published from the same booking. WSS validates
server certificates against the WebPKI root store; there is no insecure bypass.
The replacement operation is also available on the Rust WASM facade. Language
binding APIs are unchanged.

## Paid relay opt-in (Rust native and WASM)

Relay bookings remain free by default. On an explicitly enrolled organization,
configure an exact published policy, decimal slot-hour price and per-booking
credit ceiling before starting the peer:

```rust,ignore
use auki_sdk::RelayBillingAcceptance;

// Arithmetic example only: use the operator's published policy and price.
let billing = RelayBillingAcceptance::new("relay-v1", "1", "10")?;
let config = config.with_relay_billing(billing)?;
```

DMS enrollment and an NCS-backed organization allowance are also required; a P2P
credential or network-credit balance does not authorize payment on its own.
Amounts are positive decimal strings with at most six fractional places. Each
ready external slot is billed by DMS elapsed reservation time, including idle
time. TCP/WSS to one provider count as one slot. Same-org and unfilled slots earn
no charge, although the booking can hold credit for possible later assignments.

The runtime pins the accepted policy/price/ceiling and refuses to adopt an active
booking with different billing terms. Renewal keeps the original ceiling;
`payment_required` stops further renewal without discarding still-valid funded
authority. An expired paid create replay never silently buys a replacement with
a fresh ceiling. Starting a new paid booking is a new explicit application
decision; the ceiling is not a lifetime cap across separate peer starts.

Use a compatible DMS/DDS/NCS rollout before enabling this option. Python, Swift,
Web and Expo binding APIs have no new paid setting in this change and continue
to omit billing acceptance. Native and WASM Rust facades support it. The SDK
emits no traffic receipts or authoritative charge reports.
