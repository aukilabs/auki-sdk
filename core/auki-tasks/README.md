# auki-tasks

Run compute and robot task handlers with a managed DMS lease lifecycle.
This native crate handles registration, authentication, polling, heartbeats,
and results. DMS selects work; application handlers execute it.

Use the `auki-sdk` facade for Rust applications and optional P2P, or the
[Python binding](../bindings/python/auki-sdk-py/README.md) for asyncio handlers.
Start with [Run compute and robot tasks](../../docs/how-to/run-compute-tasks.md).
The [task reference](../../docs/reference/tasks.md) covers lifecycle rules,
events, failure receipts, and native runner adapters.

Run local fixture tests from the repository root:

~~~sh
cargo test --locked -p auki-tasks -p auki-dms -p auki-auth
~~~
