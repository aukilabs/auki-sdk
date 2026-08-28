# auki-sdk

`auki-sdk` is the future mechanical runtime facade for authenticated Auki
peers. It will compose identity, authentication, Domain participation,
configured routes, credential renewal, and relay allocation behind a small
host-facing lifecycle.

The crate is not a public facade yet. Its current implementation contains a
private, strict HTTP client for the DMS relay-booking contract and the runtime
coordinator that reconciles those bookings with Domain-owned Circuit Relay v2
reservations. Confirmed relay routes are fenced in a local route catalog and
are removed on assignment changes, transport loss, authority expiry, or
bounded shutdown.

The coordinator does not publish routes, discover peers, gate product work, or
expose the underlying P2P node. A later commit will compose these mechanics
with authentication and Domain lifecycle behind an `AukiPeer` API.
