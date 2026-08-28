# auki-sdk

`auki-sdk` is the future mechanical runtime facade for authenticated Auki
peers. It will compose identity, authentication, Domain participation,
configured routes, credential renewal, and relay allocation behind a small
host-facing lifecycle.

The crate is not a public facade yet. Its current implementation contains a
private authority supervisor, a strict HTTP client for the DMS relay-booking
contract, and the runtime coordinator that reconciles those bookings with
Domain-owned Circuit Relay v2 reservations.

The authority supervisor accepts either an `auki-auth` prepared peer with one
owned renewal loop or complete authority replacements from an external host.
It pins the Domain and Peer, installs verification keys before credentials,
publishes only a sensitive revisioned relay header, coalesces DMS 401 refresh
requests, and fences authorization at literal credential expiry. Authentication
and product-specific heartbeat policy remain outside this crate.

Confirmed relay routes are fenced in a local route catalog and are removed on
assignment changes, transport loss, authority expiry, or bounded shutdown.

The coordinator does not publish routes, discover peers, gate product work, or
expose the underlying P2P node. A later commit will compose these mechanics
with authentication and Domain lifecycle behind an `AukiPeer` API.
