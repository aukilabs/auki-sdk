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
