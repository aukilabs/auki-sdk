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

## Relay billing

When the backend enables relay billing, existing peer configuration books relay
capacity using the organization's available network credits. No SDK billing
option, price acceptance, per-booking cap, or language-binding change is needed.
DMS selects the price and retains it for the booking. Existing SDKs can still
book successfully; older versions may surface HTTP 402 as a generic failure.

Ready external reservations are charged by elapsed time, including idle time.
TCP and WSS routes to one provider count as one slot. Same-organization and
unfilled service incur no charge. Public bookings can still hold credit for
possible external assignments; unused holds are returned when the booking ends.
Dedicated bookings remain free.

`payment_required` on create means there is insufficient available credit for
funding. On renewal, native and WASM runtimes stop requesting extensions and keep
polling/using the existing funded authority until its deadline. Authentication
refresh does not solve a balance rejection. The application can top up the
organization and start again through its existing peer lifecycle. The SDK emits
no usage receipts or authoritative charge reports.
