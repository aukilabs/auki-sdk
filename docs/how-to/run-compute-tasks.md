# Run compute and robot tasks from Python or Rust

`AukiDmsTasks` runs handlers on an already-provisioned compute node or robot. Rust owns
registration, authentication, claiming, heartbeats and completion. Python
handlers run on the application's asyncio event loop, including its context
variables. DMS schedules work; the SDK does not submit jobs or provision nodes.

Native Rust and Python support HTTP-only tasks and optional task P2P. Robots can
also read and stay connected to their assigned Domain while idle. Web task bindings are deferred.
Existing User/App peer and data APIs remain available. There is no dependency on
the Posemesh repository, executable or runners.

## Python

Build in a virtual environment using the [binding README](../../core/bindings/python/auki-sdk-py/README.md).
Tasks work with both default and `--no-default-features` builds.

```python
import auki_sdk

async def handle(task):
    data = task.data()
    content = await data.read(task.meta["input_id"])
    await task.progress({"phase": "writing"})
    saved = await data.write(
        content.upper(), name=f"result-{task.id}", data_type="example.text.v1"
    )
    return {"output_cids": [saved["id"]], "meta": {"data_id": saved["id"]}}

# Load these values from the operator's credential storage/environment config.
credential = auki_sdk.AukiComputeCredential(
    dds_url=dds_url, dms_url=dms_url,
    registration=registration_credential, wallet_key=wallet_private_key,
    version="1.0.0", client_id=persisted_client_id,
)
tasks = auki_sdk.AukiDmsTasks(credential, {"/example/uppercase/v1": handle})
try:
    await tasks.run()
finally:
    try:
        await tasks.close()
    finally:
        await credential.close()
```

Construction makes no network requests; optional peer setup loads or creates its
private identity file. Running registers capabilities, authenticates
and polls. A compute credential has one task-runtime owner; share that runtime
rather than creating another execution owner. Registration repeats periodically;
terminal failure stops authority. Transient registration failures have at most
three attempts. Authentication renews on demand through the serialized token manager.

`run_once()` executes at most one task and returns `"completed"`, `"no_work"`
or `"busy"`. Supply `capability=` with multiple handlers. `run()` polls the
registered capabilities serially with bounded request times and a delay between
rounds. Concurrent managed `run()` calls are rejected.

Handlers return `None` or the existing DMS `output_cids`/`meta` dictionary.
A Python exception is re-raised locally. By default it reports a generic failure
to DMS; use `task.set_failure(reason, details)` to supply an explicit receipt as
described below. Exception text is not automatically sent to services.
Handler/service failures stop the managed loop.
Progress replaces its previous value. Progress and Python result metadata are
limited to 64 KiB; DMS envelopes to 2 MiB. Only HTTP 401 is replayed once after
renewal. Other claim/complete/fail failures may have an unknown outcome.

`TaskRuntimeError.kind` distinguishes configuration, authentication, authority,
lease loss, cancellation, busy/closed state, HTTP, service and data setup errors.
HTTP failures retain `status`; data operations retain `DomainDataError`.
Peer shutdown failures use kind `cleanup`. They are retained and reported by
`tasks.close()` even if the Python run awaitable was cancelled. Rust callers
must handle the `Result` returned by `AukiDmsTasks::close()`.

## Events, current task tokens and failure receipts

These features work in both managed `run()` and `run_once()`. Handlers do not
need a custom heartbeat, token-renewal or result-reporting loop.

`await task.log_event(value)` appends JSON to the next heartbeat's `events`
array. Events keep their enqueue order, including calls through cloned Rust
contexts. The queue accepts at most 1024 events and 64 KiB of serialized JSON;
overflow returns an error without dropping previously accepted events. Progress
still replaces its previous value independently of events.

Managed execution awaits an in-flight heartbeat before the final flush and
completion/failure receipt. Only acknowledged events leave the queue. A failed
heartbeat ends the lease; there is no retry on an unknown network outcome.
HTTP 401 still permits one authenticated replay, so remote delivery is not
exactly-once. DMS applies its own configured event-retention limit.

For existing Domain HTTP integrations, `task.access_token.get()` synchronously
reads the current **task Domain bearer**. Retain the handle and call `get()` before
each request: the sole heartbeat owner rotates its value. Reads fail after lease
expiry, cancellation, runtime shutdown or task completion/failure. This is not a
machine or P2P token. Prefer `task.data()` for new integrations because it also
handles Domain Server changes, 401 renewal and transfer cleanup. Never log or
cache plaintext tokens; a previously returned copy can remain valid remotely
until expiry.

`await task.set_failure(reason, details)` prepares a receipt for a handler that
subsequently raises. It does not stop execution or immediately send a request.
Use `details` for application metadata, including any uploaded artifacts:

```python
async def handle(task):
    artifacts = []
    try:
        await task.log_event({"phase": "started"})
        content = await task.data().read(task.meta["input_id"])
        saved = await task.data().write(
            content.upper(), name=f"result-{task.id}", data_type="example.text.v1"
        )
        artifacts.append({"id": saved["id"], "metadata": {"bytes": len(content)}})
        await task.log_event({"phase": "finished"})
        return {"output_cids": [saved["id"]], "meta": {"artifacts": artifacts}}
    except Exception:
        await task.set_failure("uppercase processing failed", {
            "job": {"task_id": task.id}, "artifacts": artifacts,
        })
        raise
```

The reason must be nonblank and at most 4096 UTF-8 bytes; details are limited to
64 KiB. Send only application-approved text/metadata, without credentials or
private exception dumps. A later call replaces the prepared receipt. Successful
handlers ignore it. Cancellation or lost authority skips both completion and
failure reporting, including when the handler had already prepared a receipt.
A receipt already sent to DMS before cancellation cannot be retracted; its remote
outcome may be unknown.

Rust exposes the same operations directly on `TaskContext`: `log_event(value)`,
`set_failure(reason, details)` and `access_token.get()` are synchronous `Result`
APIs. Set the receipt, finish cleanup and return `Err(TaskError::Handler)`.
The getter returns a redacted, zeroizing `SecretString`; use `expose_secret()`
only at the HTTP handoff. There is no additional refresh owner in either binding.

## Robots and idle reads

Create `AukiRobotCredential` with provisioned robot registration credentials,
`version`, `client_id`, `dds_url`, `dms_url`, the deployment's exclusive robot
`audience`, and a `capabilities` list matching the handlers. There is no wallet.
Pass it to the same `AukiDmsTasks` constructor and use the same async handlers.
The [Python robot example](../../core/bindings/python/auki-sdk-py/examples/robot_task.py)
and [Rust robot example](../../core/auki-sdk/examples/robot_task.rs) show the complete setup.

`await robot.assigned_domain_id()` registers/authenticates and returns a Domain
ID or `None`. An unassigned robot continues registration presence but does not
claim tasks. Assignment and machine identity remain pinned for the lifetime of
the credential; a changed assignment requires shutdown and a fresh credential.
DDS still verifies the persisted assignment on each token exchange.

```python
domain_id = await robot.assigned_domain_id()
if domain_id is not None:
    idle_data = robot.data(domain_id)
    try:
        configuration = await idle_data.read(configuration_id)
    finally:
        await idle_data.close()
```

Idle data uses DDS `/internal/v1/auth/robot/domain-token` and exactly `domain:r`.
Buffered writes, streaming uploads and deletes fail locally before sending data.
Use `task.data()` for task writes: its authority comes from the current lease.
The same credential owns serialized robot token renewal and the Domain-grant
cache. A 401 allows one renewal/replay; a 403 stays a denial. Robot registration
repeats periodically; registration/authentication failure closes local authority.
Close idle clients before closing the robot credential.

## Optional task P2P

In Python, add `peer_identity_file="./state/worker.identity"` to either machine
credential. Protect this file and keep it across restarts; it is separate from a
compute wallet. Optional `peer_config=` accepts the existing `AukiPeerConfig` for
listeners, direct routes, relays and DDS discovery. The default books a relay;
discovery stays off unless explicitly configured. DDS/DMS/discovery endpoints
must match the machine's environment.

The SDK binds the Peer ID during every machine login before claiming work.
Compute peer credentials come from the DMS lease/heartbeat. Robot peer credentials
come from the DDS assigned-Domain P2P exchange. One runtime-owned renewal driver
keeps robot authority current while idle or busy, independently of DMS heartbeats.
Signed credentials must match the machine type, Peer ID, Domain, issuer,
audience, literal expiry and required `domain-data:r` scope. Verification keys
and credentials rotate through the existing peer authority supervisor.
Missing peer authority fails the task startup instead of silently disabling P2P.

Inside a Python handler, `task.peer()` returns an `AukiPeer` view, or `None` for
HTTP-only tasks. Existing Info/Message/Blob/Stream adapters can use this view
when those optional protocols are compiled into the binding. The task runtime
owns peer shutdown; calling `shutdown()` on the view is rejected. Compute task
completion or cancellation fences retained peer views and awaits transport,
discovery and relay cleanup. A robot task borrows the runtime's persistent peer;
completing, failing or cancelling that task leaves the robot connected.

Call `await tasks.start()` to register and start robot networking before polling.
Then `tasks.peer()` returns its peer, or `None` for HTTP-only/unassigned robots.
`run()` and `run_once()` also start it automatically. Startup is shared across
callers and remains runtime-owned if a caller cancels its awaitable; await
`tasks.close()` to stop and drain it. Compute `tasks.peer()` is always `None`;
its peer is available only through the active task.

```python
await tasks.start()
peer = tasks.peer()  # Robot peer: usable before and between tasks.
await tasks.run()   # Reuses the same connection and renewal owner.
```

Applications own protocol registration lifetimes. Close task-specific endpoints
in the handler's `finally` block; close process-wide endpoints before runtime
shutdown. Retained robot peer views remain usable between tasks, while retained
task data clients lose access when their lease ends. Task completion never grants
idle writes. Robot authority/assignment loss fences the peer and stops work;
changing assignment requires a fresh runtime and credential.

Rust uses `AukiTaskPeerConfig`, its `identity_proof()` in the machine config,
and `AukiDmsTasks::new_with_peer`. Import `TaskPeerContext` to call `task.peer()`
or `tasks.peer()` after `tasks.start(&cancellation).await?`;
the returned view exposes the existing protocol context without authority controls.
The Rust robot example shows both HTTP-only and P2P construction.

Compute task heartbeats and the robot authority driver each service their own
peer/relay refresh requests. There is no P2P scheduler or task dispatch protocol.
Await `tasks.close()` to stop the robot peer and drain its discovery, relay and
transport cleanup. Close the machine credential last. Application operations
still require application authorization.

## Cancellation and authority

Cancelling the run awaitable requests native cancellation. `close()` cancels
active work and awaits the actual Python handler's `finally` blocks, data cleanup
and worker registration/peer shutdown. Keep the asyncio loop alive until it returns.
Call close from the host, not the handler it is waiting for. Close is repeatable,
including after cancellation of a previous close awaitable.

Lease loss, DMS cancellation, invalid rotations and registration failure cancel
the handler and skip normal completion. Retained task data clients stop at task
end. Data grants validate Domain, issuer, server audience and expiry against the
authenticated DMS envelope; Domain Servers verify signatures and permissions.
Already-issued bearer tokens may remain valid remotely until expiry.

A data 401 asks the sole heartbeat owner for renewal; the data client never
starts another heartbeat loop. Transfers keep their [streaming/cleanup limits](domain-data.md).
Threads, CPU-heavy functions, subprocesses and hardware need application-owned
cooperative stopping. The SDK waits for cleanup before releasing the execution
slot. DMS attempts can repeat side effects after lease loss; use application/task
operation IDs where needed.

The [Python example](../../core/bindings/python/auki-sdk-py/examples/compute_task.py)
handles SIGINT/SIGTERM. It registers, claims real work and creates result data.
Run only with approved endpoints, provisioned credentials, task/Domain scope and
cleanup. The job submitter owns result cleanup. Tests use loopback fixtures.

## Adapting an existing native host

Use `claim_any()` to preserve DMS selection across the registered capability set,
then `TaskLease::execute()` to retain managed heartbeats, events, reporting and
awaited cleanup. This lets a host use one cancellation token to stop claiming
and another to interrupt active work. Close the runtime after the lease ends.

For hosts serving both HTTP-only and P2P compute tasks, explicitly choose
`execute_with_optional_peer()`: an absent grant leaves `task.peer()` empty;
a partial, expired or invalid grant still fails. Ordinary `execute()` and
`run_once()` retain their requirement for peer authority when P2P is configured.
Robot peers keep their runtime lifetime. A runner needing P2P must require a
peer before starting its work.

`task.credential.lease_snapshot()` supplies the current Domain bearer and initial
heartbeat metadata to a legacy Rust runner. Do not log or report this snapshot;
P2P credentials are removed. Continue reading `task.access_token.get()` for
later HTTP requests. `AukiTaskPeer::subscribe_status()` permits a host to pause
claims or stop application work when peer authority or relay readiness is lost.
These adapters add no Python custom lease API or backend wire changes.

## Rust and existing Posemesh hosts

The SDK reexports `AukiComputeCredential`, `AukiDmsTasks`, `TaskHandler`,
`TaskContext`, `TaskCredential` and `TaskResult` from native `auki-tasks`.
The [Rust example](../../core/auki-sdk/examples/compute_task.rs) executes at most
one task. Handlers observe `task.cancellation()`/`task.cancelled()` and finish
cleanup before returning.

Custom loops call `claim()` and the returned `TaskLease`'s `heartbeat()`,
`complete()` and `fail()`. They own heartbeat timing; there is no automatic loop.
With P2P configured, call `start_peer()` after the first heartbeat. Await
`lease.close()` on cancellation; it releases local resources without completing
or failing the remote task. Release the lease before awaiting runtime close.
Drop revokes local data access and initiates best-effort peer cleanup;
await cancellation for orderly cleanup because dropped Rust futures cannot run
async cleanup. The managed runner waits for handler cleanup on cancellation.

`AukiDmsTasks::from_client` accepts an existing authenticated `DmsClient` for host
adapters. The host must stop this runtime when its external authority ends.
Existing Posemesh runners/storage ports are not removed or migrated automatically.

**Required with a Posemesh SDK-pin update:** `lease_by_capability` now sends its
filter. Posemesh at `37db36a` passes its first capability and relied on the ignored
filter. Change `acquire_lease_with_dms` to `lease_any()` so DMS can select across
all authorized capabilities, or migrate to the new runtime. Coordinate this
caller change with the dependency update and test multiple-capability dispatch.
The current Posemesh checkout remains pinned to its earlier SDK revision.

Follow-up in #375: adapt existing Runner/input/output conventions to this
lifecycle. Python exposes managed task handlers. Application runners,
configuration and hardware control stay outside the generic SDK lifecycle.
Stable crates have no dependency on Posemesh or `labs/` runners.

Contracts stay on DDS SIWE/register-wallet and DMS task/heartbeat/complete/fail
routes. No provider or infrastructure deployment is required for this addition.
Live support still depends on deployed versions and admission settings; fixtures
do not establish deployed machine compatibility.

Provider evidence: DDS robot registration/domain/P2P contracts at `b27c0804`
and DMS `8088111` (compute task P2P across registered capabilities). Older DMS
revisions issue compute P2P credentials only for specific built-in capabilities.
Robots additionally require `DDS_ROBOT_WORKERS_ENABLED` in DDS,
`ROBOT_WORKERS_ENABLED` in DMS and the same exclusive `DDS_ROBOT_AUDIENCE`.
Local tests exercise these contracts and real local peer exchanges; live machine
execution and deployment flags have not been validated by this change.
Ordered events and explicit failure receipts use the existing DMS `events`,
`reason` and `details` fields, also checked at `004688cb`. These SDK APIs are
additive and do not change provider routes or payload formats.
