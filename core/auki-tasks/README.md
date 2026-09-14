# Native task lifecycle

`auki-tasks` manages one DMS lease for Rust or Python handlers. It reuses
`auki-auth` machine authentication, `auki-dms` wire operations and
`auki-domain-client` data transfers. DMS decides which machine receives work.

Supports already-provisioned compute and robot credentials, HTTP-only handlers,
assigned-robot idle reads and optional P2P through the SDK peer adapter.
Compute peers follow leases; robot peers remain connected between tasks.
Web task execution is not exposed.

`start()` registers/authenticates without claiming work and starts an assigned
robot's peer. The SDK's `TaskPeerContext` trait exposes `tasks.peer()` for idle
robot use. `run()`/`run_once()` start it automatically. Robot DDS peer renewal
has one runtime-owned driver independent of task heartbeats. Await runtime close
to drain peer, discovery and relay cleanup. Applications close task-specific
protocol registrations when their handlers finish.

`AukiDmsTasks::run` manages polling and heartbeats; `claim` returns a `TaskLease`
for custom loops using the same `heartbeat`, `complete` and `fail` operations.
Custom loops own their heartbeat timing and must release their lease before
awaiting runtime close. Each runtime permits one live lease across its clones.
With P2P enabled, call `start_peer()` after the initial heartbeat; await
`lease.close()` to cancel local work without reporting a remote result.

Handlers receive task metadata, renewable `TaskCredential`, a selected data
client and cancellation. They must stop their work and await cleanup before
returning. CPU work, threads, subprocesses and hardware require their own stop
mechanism. Async Python handlers receive cancellation on their event-loop task;
close waits for their `finally` blocks. Retained data clients stop when a lease
ends. Already-issued remote bearer tokens may remain valid until expiry.
Runtime `close()` returns a `Result` and retains peer shutdown failures so a
cancelled run cannot hide a failed relay/transport cleanup.

Managed handlers call `task.log_event(value)` for ordered heartbeat events and
`task.access_token.get()` for the latest task Domain HTTP bearer. The token
handle shares rotation/revocation with the lease; it never refreshes independently.
Prepare an explicit DMS failure reason and artifact metadata with
`task.set_failure(reason, details)`, then finish cleanup and return an error.
Managed execution drains the in-flight heartbeat and final event batch before
reporting. Cancellation/authority loss skips receipts. See
[limits and examples](../../docs/how-to/run-compute-tasks.md#events-current-task-tokens-and-failure-receipts).

The runtime uses current `meta`, `inputs_cids`, `output_cids` and completion
metadata without defining a new task payload or scheduler. Handler errors stop
the managed loop after reporting failure. DMS controls task retries; there is
no exactly-once guarantee. Network failures during claim/complete/fail can have
an unknown outcome and are not automatically replayed except once after 401.

Run local validation from the workspace root:

```sh
cargo test --locked -p auki-tasks -p auki-dms -p auki-auth
```

Provider contracts: DDS `/internal/v1/auth/siwe/{request,verify}` and
`/internal/v1/nodes/register-wallet`; DMS `/tasks` and
`/tasks/{id}/{heartbeat,complete,fail}`. Configuration must identify one intended
environment. Construction does no I/O; running registers capabilities and claims
shared work. Follow the approved environment/account/operation scope before
running a live example.
