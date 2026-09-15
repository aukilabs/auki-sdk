# Run compute and robot tasks

Use `AukiDmsTasks` to run Python or Rust handlers on a provisioned compute node
or robot. The SDK registers capabilities, claims DMS leases, sends heartbeats,
and reports results. Your handler executes the task. It can use Domain data
and optional P2P without a dependency on Posemesh.

Before starting, build the [Python binding](../../core/bindings/python/auki-sdk-py/README.md)
or [add the Rust SDK](../reference/networking.md#rust). Get the machine's
credentials and matching DDS/DMS URLs from its operator. Compute nodes need
a registration credential and wallet key; robots use a robot registration
credential. The SDK selects the robot audience for official Auki DDS URLs;
custom endpoints need an explicit value. Check the
[service requirements](../reference/tasks.md#service-requirements) for your environment.

## Try the paired example

The [compute + robot Python example](../../core/examples/compute-robot/README.md)
walks through run-isolated HTTP input/output, an uppercase compute task and a
robot inspection report. It includes offline tests, bounded worker commands,
and operator-owned verification and cleanup guidance. Use only provisioned
workers and explicitly approved Domain/job/data scope.

## Write a Python handler

A handler is an async function registered under a DMS capability. This example
expects the task's `meta` to contain an `input_id` for a UTF-8 text record:

~~~python
async def uppercase(task):
    data = task.data()
    content = await data.read(task.meta["input_id"])
    await task.progress({"phase": "writing"})
    saved = await data.write(
        content.decode("utf-8").upper().encode("utf-8"),
        name=f"result-{task.id}",
        data_type="example.text.v1",
    )
    return {"output_cids": [saved["id"]], "meta": {"data_id": saved["id"]}}
~~~

Return `None` for a task with no result, or a dictionary containing `output_cids`
and optional `meta`. Raising an exception reports failure and stops the managed
loop. The exception is re-raised locally; its text is not sent to DMS.

## Start a compute node

Load credentials from your environment or credential store. Keep the same
`AUKI_CLIENT_ID` across restarts:

~~~python
import os
import auki_sdk

async def run_compute():
    credential = auki_sdk.AukiComputeCredential(
        dds_url=os.environ["DDS_BASE_URL"],
        dms_url=os.environ["DMS_BASE_URL"],
        registration=os.environ["NODE_REGISTRATION_CREDENTIAL"],
        wallet_key=os.environ["NODE_WALLET_KEY"],
        version="1.0.0",
        client_id=os.environ["AUKI_CLIENT_ID"],
    )
    tasks = auki_sdk.AukiDmsTasks(credential, {"/example/uppercase/v1": uppercase})
    try:
        await tasks.run()
    finally:
        try:
            await tasks.close()
        finally:
            await credential.close()
~~~

Call `run_compute()` from your application's asyncio loop. `run()` polls the
registered capabilities until cancelled or an error occurs. To attempt one task,
use `await tasks.run_once()`; with multiple handlers, supply `capability=`.
The outcome is `"completed"`, `"no_work"`, or `"busy"`.

The [complete Python example](../../core/bindings/python/auki-sdk-py/examples/compute_task.py)
also handles SIGINT and SIGTERM. It registers a capability, claims real work,
and creates result data. Use approved endpoints, credentials, and task/Domain
scope when running it. The task submitter owns result cleanup.

## Report events and failure details

Use `task.progress(value)` for the latest status and `task.log_event(value)`
for an ordered sequence of events. The SDK sends both in heartbeats.

To include a reason and artifact metadata in a failure receipt, call
`set_failure` before raising. For example, a handler that tracks its uploaded
artifacts can report them even if later processing fails:

~~~python
async def handle(task):
    artifacts = []
    try:
        await task.log_event({"phase": "started"})
        result = await process(task, artifacts)
        await task.log_event({"phase": "finished"})
        return result
    except Exception:
        await task.set_failure("processing failed", {"artifacts": artifacts})
        raise
~~~

Here, `process` is your application function; it appends metadata to `artifacts`
after each successful upload. `set_failure` prepares the receipt without ending
the task. A successful handler ignores it. Cancellation or lost authority skips
completion and failure reporting. See [events and receipts](../reference/tasks.md#events-and-receipts)
for limits and delivery behavior.

For an existing HTTP client, call `task.access_token.get()` immediately before
each request to read the current task Domain bearer. Keep the handle rather
than a copied token: heartbeats rotate its value, and reads fail after the task
ends. Prefer `task.data()` when using the SDK data client. See
[task tokens](../reference/tasks.md#current-task-token) for the Rust equivalent.

## Use a robot

Pass `AukiRobotCredential` to the same runtime. Set `capabilities` to match
the handler keys; no wallet is required:

~~~python
robot = auki_sdk.AukiRobotCredential(
    dds_url=os.environ["DDS_BASE_URL"],
    dms_url=os.environ["DMS_BASE_URL"],
    registration=os.environ["ROBOT_REGISTRATION_CREDENTIAL"],
    audience=os.environ.get("DDS_ROBOT_AUDIENCE"),
    version="1.0.0",
    client_id=os.environ["AUKI_CLIENT_ID"],
    capabilities=["/example/uppercase/v1"],
)
tasks = auki_sdk.AukiDmsTasks(robot, {"/example/uppercase/v1": uppercase})
~~~

Omit `audience` to use the default for Auki dev, staging, or production. Set it
explicitly for a custom DDS endpoint or a deployment with a different audience.
See [robot audience defaults](../reference/tasks.md#robot-audience).

Use the same run and cleanup pattern as for a compute node. An unassigned robot
reports presence but does not claim tasks. An assigned robot can also read
Domain data while idle:

~~~python
domain_id = await robot.assigned_domain_id()
if domain_id is not None:
    idle_data = robot.data(domain_id)
    try:
        configuration = await idle_data.read(configuration_id)
    finally:
        await idle_data.close()
~~~

`configuration_id` is a record selected by your app. Idle access is read-only;
use `task.data()` for writes authorized by a lease. Close idle clients before
closing the robot credential. Assignment changes require a fresh credential
and runtime. The [Python robot example](../../core/bindings/python/auki-sdk-py/examples/robot_task.py)
shows the full setup.

## Connect a task or robot peer

Add `peer_identity_file="./state/worker.identity"` to either credential's
constructor. Protect this file and retain it across restarts. Use `peer_config=`
to configure listeners, relays, and discovery. By default, the peer books a relay
and discovery is disabled; keep all endpoints in the machine's environment.

Inside a handler, `task.peer()` returns a peer view, or `None` for HTTP-only
work. Use it with the protocols compiled into your binding. A compute peer
closes when its task ends. A robot peer stays connected between tasks.

To start a robot peer before polling, await `tasks.start()` and get its view
with `tasks.peer()`. `run()` and `run_once()` also start it automatically.
`tasks.peer()` returns `None` for compute nodes and robots without an assigned,
configured peer. See [peer lifetime](../reference/tasks.md#authority-and-peer-lifetime).

The runtime owns peer shutdown. Close task-specific protocol endpoints in the
handler's `finally` block and process-wide endpoints before `tasks.close()`.
Registering a P2P handler does not register a DMS capability or authorize the
application operations it serves.

## Stop work cleanly

Cancel the Python run task, then await `tasks.close()` from the host. Keep the
asyncio loop alive until close finishes: it waits for handler `finally` blocks,
data transfers, registration, and peer cleanup. Close the credential last.

Lease loss and authority failure also cancel the handler. CPU work, threads,
subprocesses, and hardware need an application stop mechanism that the handler
awaits before returning. The SDK holds the execution slot until cleanup finishes.
DMS may retry work, so use task or operation IDs to protect side effects that
must not be repeated.

## Use Rust or an existing runner

Implement `TaskHandler::run` with a `TaskContext` and return a `TaskResult`.
Observe `task.cancellation()` or await `task.cancelled()`, then finish cleanup
before returning. Check the result of `tasks.close().await` before exiting.

The [Rust compute example](../../core/auki-sdk/examples/compute_task.rs) executes
at most one task; the [Rust robot example](../../core/auki-sdk/examples/robot_task.rs)
also shows P2P setup. Existing native runners can use `claim_any()` followed
by managed `lease.execute()` to separate stopping claims from interrupting
active work. See [native runner adapters](../reference/tasks.md#native-runner-adapters).
