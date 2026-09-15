# Paired compute + robot workers (Python)

A small **HTTP-first** example using SDK-managed DDS registration and DMS tasks:

1. An operator uploads a small UTF-8 input and submits a dedicated compute task.
2. `compute.py` reads it through `task.data()` and writes uppercase UTF-8 bytes.
3. The operator submits a dedicated robot task with that output as its input.
4. `robot.py` writes a JSON inspection report. It does not control hardware.

DMS remains the scheduler. The SDK owns polling, leases, heartbeats, token renewal,
results and awaited shutdown. No Posemesh runner or new provisioning is required.
These workers **do contact real services when started**. The tests are offline.

## Build and configure

Use Python 3.9+ and build the [Python SDK](../../bindings/python/auki-sdk-py/README.md)
inside a virtual environment. From the repository root:

```sh
python3 -m venv .venv
. .venv/bin/activate
python -m pip install 'maturin>=1.5,<2.0' pytest
maturin develop --locked --no-default-features --manifest-path core/bindings/python/auki-sdk-py/Cargo.toml
```

The following values are placeholders, not usable credentials or Domain IDs.
Load real machine secrets through your approved secret store, not command-line
arguments or checked-in files. Both workers share these non-secret settings:

```sh
export DDS_BASE_URL='https://dds.dev.aukiverse.com'
export DMS_BASE_URL='https://dms.dev.aukiverse.com/v1'
export AUKI_DOMAIN_ID='<approved-domain-uuid>'
export AUKI_RUN_ID='<fresh-canonical-lowercase-uuid-for-this-run>'
```

In the compute process environment, set:

```sh
export AUKI_CLIENT_ID='compute-example-installation'
export NODE_REGISTRATION_CREDENTIAL='<provisioned-compute-credential>'
export NODE_WALLET_KEY='<provisioned-compute-wallet-key>'
python core/examples/compute-robot/compute.py --once --timeout 180
```

In a separate robot process environment, set:

```sh
export AUKI_CLIENT_ID='robot-example-installation'
export ROBOT_REGISTRATION_CREDENTIAL='<provisioned-robot-credential>'
# For official dev, omit DDS_ROBOT_AUDIENCE to use the SDK default.
# Custom DDS endpoints require an explicit deployment-specific audience.
python core/examples/compute-robot/robot.py --once --timeout 180
```

Client IDs must be stable across restarts and **distinct per worker**. Never give
the robot the compute wallet or use a human/App token as a registration credential.
The robot must already be assigned to the selected Domain. Keep service endpoints
in the same approved environment; there are no implicit dev/prod fallbacks.

After SDK startup each process prints JSON with `event: "ready"`, `role` and
`run_id`. This is startup readiness, not proof of a completed job. `--once` polls
through `no_work`/`busy` until one task completes; its timeout includes startup,
polling and execution. Cleanup is awaited even after timeout and can extend past
the deadline. Omit `--once` to keep running until SIGINT/SIGTERM. A failure exits
nonzero with a fixed JSON error, never an exception body or token.

## Task and artifact contract

Use only these run-specific capabilities:

```text
/examples/compute-robot/{run_id}/compute/v1
/examples/compute-robot/{run_id}/robot/v1
```

Each task must target `AUKI_DOMAIN_ID` and have:

```json
{"input_id": "<domain-data-id>", "run_id": "<same-run-uuid>"}
```

Handlers reject the wrong Domain, wrong run or missing input before data I/O.
Artifacts are named `sdk-{run_id}-{role}-{task.id}`. Named writes reject duplicates;
this is not an exactly-once application. The helper sets `max_attempts=1` and
cleanup enumerates matching run-owned names. Each handler emits `started`/`finished`
events and `reading`/`writing` progress.

Both return `output_cids: [id]` and `meta` containing `data_id`, `run_id`, `sha256`
and `bytes`. Compute metadata describes the **stored uppercase output**. Robot
metadata describes the **inspected input**, not the serialized report. The stored
robot report is JSON containing `input_id`, `bytes` and `sha256` of that input.

The operator-side helper is [`test-support/run-compute-robot.py`](../../../test-support/run-compute-robot.py).
Keep its environment separate from the workers. Set the shared run/Domain/DMS
values above, plus `DOMAIN_SERVER_URL`, `AUKI_DOMAIN_TOKEN_FILE`,
`AUKI_COMPUTE_ID` and `AUKI_ROBOT_ID`. The latter are provisioned DDS worker UUIDs,
not Peer IDs. The token file must be private and contain a fresh human-authorized
Domain token with `domain:rw`, issued for the selected Domain. Human login and
token exchange belong to the operator; never pass the human password to workers.
The helper accepts only official dev DMS and the dev filesystem/S3 data servers.

```sh
# Offline: no credential reads or network calls.
python test-support/run-compute-robot.py
# After both workers are polling, using your small UTF-8 input file:
python test-support/run-compute-robot.py --live --input-file input.txt --report /private/new-run-report.json
```

The report path must not exist; it is created with mode 0600 before mutations.
The helper estimates availability before submission, creates dedicated compute
then robot jobs, checks the actual executors and raw output bytes, and removes
its named data only on success. DDS `tasks.start()` alone does not announce DMS
availability: workers must poll. On failure it requests cancellation only for
known run-owned jobs, retains data, and reports reconciliation identifiers. Stop
the workers before manual cleanup. If a POST response was lost, find the job by
the report's run UUID/`sdk-{run_id}-{role}` label before retrying; writes are not
automatically replayed. Existing worker records and terminal jobs are retained.

## Importable setup and optional peers

Add this directory to `sys.path`, then use
`common.build_worker(role, env=None) -> (credential, tasks)`. `env` is a mapping;
omitting it uses `os.environ`. Roles are exactly `compute` and `robot`. The
handlers are `compute.handle(task, domain_id, run_id)` and
`robot.handle(task, domain_id, run_id)`; setup binds the configured scope.

A host can await `tasks.start()` and drive the SDK runtime itself, or await
`common.run_worker(credential, tasks, role, run_id, once=True, timeout=120)`.
The latter owns cleanup; otherwise always close `tasks` before `credential` in
nested `finally` blocks. Do not retain task data/token handles after the lease.

HTTP needs no peer identity or protocol feature. For optional P2P, build with
`--no-default-features --features info`. Create **separate private persistent
identity files** using `auki_sdk.Identity.load_or_create(path)` and read their
`peer_id` properties. In each worker environment set `AUKI_PEER_IDENTITY_FILE`
to its own file and `AUKI_EXPECTED_PEER_ID` to its partner's ID. Do not print keys.

The default peer configuration is direct-only on loopback, suitable for running
both workers on this host. No discovery is enabled. With explicit relay approval,
set `AUKI_P2P_RELAY=1` on the robot to enable its relay booking. Its ready JSON
includes the public `peer_id` and `route`. Pass them to the operator as
`AUKI_REMOTE_PEER_ID` and `AUKI_REMOTE_ROUTE`. The compute task fetches the robot's
Info marker through its lease-scoped peer and checks the exact partner/run.
The robot endpoint admits only the configured authenticated compute peer. Info's
callback receives a principal dictionary, not a bare ID. This demo returns only
its run marker; it is not an application authorization framework.

The robot endpoint stays mounted while idle/between tasks and closes before the
runtime. Compute peers stop at task end. For a one-shot paired run, use `--once`
on both workers; the robot remains idle while compute runs, then handles its own
task. See the [task guide](../../../docs/how-to/run-compute-tasks.md#connect-a-task-or-robot-peer).

## Approved operations and cleanup

Before any shared-dev run, approve the exact existing worker records, Domain,
capabilities, data writes, dedicated jobs and optional relay/discovery operations.
**Registration updates persistent worker capabilities** as well as presence and
version. Save the previous capability sets and restore them using supported
registration after stopping this run. These workers do not provision workers,
assign robots, rotate credentials, or automatically restore capabilities.

The operator must verify the actual executor identities and artifacts, cancel
only this run's unfinished jobs on failure, delete only this run's input/output
records, stop these processes and verify lease/presence/peer cleanup. Retain
terminal receipt evidence and existing worker records. Never claim generic work,
delete unrelated data, or assume process exit alone proves remote cleanup.

## Offline tests

```sh
python -m pytest core/examples/compute-robot test-support/test_compute_robot_operator.py -q
```

Tests use in-memory data and SDK boundary doubles; they require no credentials,
registration, wallet, network or compiled binding. They are not live-provider
acceptance evidence. Full SDK/provider and optional P2P checks belong to the
separate test-support harness.
