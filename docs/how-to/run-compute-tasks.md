# Run compute tasks from Python or Rust

`AukiDmsTasks` runs handlers on an already-provisioned compute node. Rust owns
registration, authentication, claiming, heartbeats and completion. Python
handlers run on the application's asyncio event loop, including its context
variables. DMS schedules work; the SDK does not submit jobs or provision nodes.

This milestone supports native compute execution over HTTP. Robot credentials,
idle robot data access and task P2P integration remain in #375. Web task bindings
are deferred. Existing User/App peer and data APIs remain available.

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

Construction makes no requests. Running registers capabilities, authenticates
and polls. A compute credential has one task-runtime owner; share that runtime
rather than creating another execution owner. Registration repeats periodically;
terminal failure stops authority. Transient registration failures have at most
three attempts. Authentication renews on demand through the serialized token manager.

`run_once()` executes at most one task and returns `"completed"`, `"no_work"`
or `"busy"`. Supply `capability=` with multiple handlers. `run()` polls the
registered capabilities serially with bounded request times and a delay between
rounds. Concurrent managed `run()` calls are rejected.

Handlers return `None` or the existing DMS `output_cids`/`meta` dictionary.
A Python exception reports a generic failure to DMS and is re-raised locally;
its text is not sent to services. Handler/service failures stop the managed loop.
Progress replaces its previous value. Progress and Python result metadata are
limited to 64 KiB; DMS envelopes to 2 MiB. Only HTTP 401 is replayed once after
renewal. Other claim/complete/fail failures may have an unknown outcome.

`TaskRuntimeError.kind` distinguishes configuration, authentication, authority,
lease loss, cancellation, busy/closed state, HTTP, service and data setup errors.
HTTP failures retain `status`; data operations retain `DomainDataError`.

## Cancellation and authority

Cancelling the run awaitable requests native cancellation. `close()` cancels
active work and awaits the actual Python handler's `finally` blocks, data cleanup
and compute registration shutdown. Keep the asyncio loop alive until it returns.
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

## Rust and existing Posemesh hosts

The SDK reexports `AukiComputeCredential`, `AukiDmsTasks`, `TaskHandler`,
`TaskContext`, `TaskCredential` and `TaskResult` from native `auki-tasks`.
The [Rust example](../../core/auki-sdk/examples/compute_task.rs) executes at most
one task. Handlers observe `task.cancellation()`/`task.cancelled()` and finish
cleanup before returning.

Custom loops call `claim()` and the returned `TaskLease`'s `heartbeat()`,
`complete()` and `fail()`. They own heartbeat timing; there is no automatic loop.
Release the lease before awaiting runtime close. Drop revokes local data access;
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
lifecycle, then integrate robot and optional P2P authority. Application runners,
configuration and hardware control stay outside the generic SDK lifecycle.
Stable crates have no dependency on Posemesh or `labs/` runners.

Contracts stay on DDS SIWE/register-wallet and DMS task/heartbeat/complete/fail
routes. No provider or infrastructure deployment is required for this addition.
Live support still depends on deployed versions and admission settings; fixtures
do not establish deployed machine compatibility.
