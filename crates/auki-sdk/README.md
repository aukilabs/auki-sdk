# auki-sdk

`auki-sdk` is the future mechanical runtime facade for authenticated Auki
peers. It will compose identity, authentication, Domain participation,
configured routes, credential renewal, and relay allocation behind a small
host-facing lifecycle.

The crate is not a public facade yet. Its current implementation contains only
a private, strict HTTP client for the DMS relay-booking contract. That client
validates request bounds, response shapes, control headers, and authorization
retry behavior before a later commit adds coordination or an `AukiPeer` API.
