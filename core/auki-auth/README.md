# auki-auth

Sign in with User or App credentials, or import a ZITADEL session, to access
an Auki Domain. `auki-sdk` uses this crate when starting a peer.

See [Sign in and choose a Domain](../../docs/how-to/authenticate.md).
User, App, and imported sessions also support [Domain data access](../../docs/how-to/domain-data.md)
without a peer. For machine authentication, see
[Run compute and robot tasks](../../docs/how-to/run-compute-tasks.md).

## Typed machine inventory

`AuthSession::inventory_nodes` and `inventory_robots` share existing API service
credentials, refresh and persistence. They expose only nonsecret inventory
metadata. Unsupported imported grant profiles return `None`; an empty supported
response returns `Some(Vec::new())`. Robot reads enforce the selected Domain and
imported allowlist. The [fleet client](../auki-fleet/README.md) composes these
reads with authorized DMS observations; inventory access grants no task authority.

`inventory_nodes_page(limit, cursor, cancellation)` and
`inventory_robots_page(domain_id, limit, cursor, cancellation)` request pages of
1–100 records. `InventoryPage` exposes `items`, an opaque `next_cursor`, and
`paginated`. A first legacy complete response has `paginated == false`; missing
acknowledgement after continuation is an error. Per-page byte/deadline bounds,
identity validation and authorization remain in force. The non-page methods
retain their legacy complete-list behavior. Callers performing their own
traversal must bound pages/time and pass cursors unchanged.
