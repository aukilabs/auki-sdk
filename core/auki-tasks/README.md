# Native task lifecycle

`auki-tasks` manages one DMS lease for Rust or Python handlers. It reuses
`auki-auth` machine authentication, `auki-dms` wire operations and
`auki-domain-client` data transfers. DMS decides which machine receives work.

The first milestone supports already-provisioned compute credentials and
HTTP-only handlers. Robot authentication/idle access and task P2P integration
remain follow-ups in #375. Web task execution is not exposed.

`AukiDmsTasks::run` manages polling and heartbeats; `claim` returns a `TaskLease`
for custom loops using the same `heartbeat`, `complete` and `fail` operations.
Custom loops own their heartbeat timing and must release their lease before
awaiting runtime close. Each runtime permits one live lease across its clones.

Handlers receive task metadata, renewable `TaskCredential`, a selected data
client and cancellation. They must stop their work and await cleanup before
returning. CPU work, threads, subprocesses and hardware require their own stop
mechanism. Async Python handlers receive cancellation on their event-loop task;
close waits for their `finally` blocks. Retained data clients stop when a lease
ends. Already-issued remote bearer tokens may remain valid until expiry.

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
