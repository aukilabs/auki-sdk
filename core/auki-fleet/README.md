# auki-fleet

Read Domain robot inventory and observed compute activity, or inspect a separate
compute candidate pool. Uses the shared User/App or imported session without
starting a peer, worker, or polling loop. DMS retains scheduling authority.

Use `AukiFleet::new(session, dms_url)?.in_domain(domain_id)` through `auki-sdk`.
Snapshots describe each source's completeness; missing activity remains unknown.
Call and await `close()` before closing the shared session.

Provider contracts and limits are documented in
[the fleet reference](../../docs/reference/fleet.md). Run local fixtures with:

~~~sh
cargo test --locked -p auki-fleet -p auki-auth -p auki-dms --features auki-dms/jobs
cargo check --locked --target wasm32-unknown-unknown -p auki-fleet -p auki-sdk
~~~
