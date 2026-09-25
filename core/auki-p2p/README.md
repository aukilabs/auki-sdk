# auki-p2p

TCP and relay transport with peer authentication. Most apps should use
[`auki-sdk`](../auki-sdk/README.md), which also handles sign-in, discovery,
and relay bookings.

See [How Auki networking works](../../docs/explanation/networking.md).

Exact-route circuits share one hop per `(target peer, circuit multiaddr)`
while any `open_exact` / `connect_relayed` owner is live. Additional
protocols on that hop open yamux streams only. Each owner may release
once, so a cancelled close plus Drop cannot tear down a sibling stream.
The last unique owner close tears the hop down so a later exact-route
open dials again.

Native inbound protocols have a bounded queue of 64 negotiated streams. A burst
beyond that queue is rejected. Managed servers authenticate concurrently;
pending handshakes and application handlers share the protocol spec's concurrency
limit. Closing a managed server cancels and awaits those tasks and closes queued
streams. Direct users of `Node::accept` must keep accepting to drain their queue.

## Stalled relay connections

Native TCP/WSS and browser transports share a bounded source-admission recovery
policy. Three negotiation timeouts for `/auki-p2p/relay-auth/1`, spanning at least
20 seconds between the first and last timeout, retire only the selected direct
connection. With sequential ten-second negotiations this is roughly 30 seconds.
Concurrent failures coalesce into one close. A successful targeted negotiation
resets the budget and fences older pending failures; timeout history also resets
after 60 seconds without another timeout. Application-protocol timeouts and
authorization denials do not trigger this policy.

Handler outcomes count even after a caller drops its open future. The SDK booking
coordinator handles the resulting reservation loss through its existing fenced
DMS recovery lifecycle; this policy does not create another booking or replay
application data. Low-level Node users must maintain their own reservation
lifecycle. Streams on the retired connection close and must be reopened by the
application. Independent connections, including another connection to the same
peer, are not selected for closure.

After a node has reserved on a relay, new source circuits through that relay
require a confirmed local reservation. This check runs before selecting/dialing
the source connection and again before acquiring a circuit. It remains in force
after cancellation completes, until a replacement reservation is confirmed.
Otherwise a new connection could be accepted under the provider's old authority
and then closed when the provider processes recovery. Opens during this window
return `RelayReservationClosed`; callers should wait for readiness before retrying.
Reservation establishment itself remains allowed so recovery can progress.
Outbound-only use of relays on which the node has never reserved is unchanged.
Low-level users that explicitly cancel a reservation must confirm a replacement
before opening another circuit through that relay, even if other connections
remain. Existing streams are not replayed or migrated by this guard.

The `auki_p2p::relay_recovery` tracing target emits safe connection IDs, peer IDs,
and bounded close-reason classifications. It excludes credentials and raw I/O
error bodies. See the [local regression fixture](../../test-support/circuit-handover/README.md).

Close diagnostics also include the base transport, nested I/O kinds, numeric OS
codes, and allowlisted error layers (up to 16). Native WSS errors distinguish
WebSocket framing, size, close/EOF and underlying I/O failures when those types
survive upstream wrapping. Unknown errors remain `other`; no raw error message,
WebSocket close reason or address is formatted. These observations do not identify
which endpoint initiated a close by themselves. Browser errors can expose less
detail than native WSS errors.
