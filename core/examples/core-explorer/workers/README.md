# Jobs Playground workers

These HTTP-only workers reuse the deterministic handlers in
[compute-robot](../../compute-robot/README.md): compute writes uppercase UTF-8;
robot writes a sorted JSON report containing input UUID, byte count and SHA-256.
The robot performs simulated inspection only, with no hardware actions.
`run.py` uses actual `AukiComputeCredential`, `AukiRobotCredential`, and
`AukiDmsTasks`. DMS owns scheduling; the SDK owns registration, leases,
heartbeats, progress/events and results. No peer, discovery or relay is needed.

**Implementation is not activation approval.** Nothing here provisions identities,
installs services, submits jobs or starts itself. Running without `--check`
contacts the configured DDS/DMS and lease-authorized Domain Server, registers
capabilities and polls work. Obtain separate approval for exact endpoints,
workers, organization/Domain, capability changes, operations and credit budget
before activation. Endpoint configuration has no environment fallback; operators
must verify DDS, DMS and the browser API belong to the same approved environment.

## Private configuration

Use Python 3.9+ on POSIX/Linux and the matching native Python SDK from the
[Python binding README](../../../bindings/python/auki-sdk-py/README.md).
No native SDK or third-party Python packages are needed for `--check` or tests.
The two `*.example.json` files contain intentionally invalid placeholders.
Do not put real credentials in this repository, browser storage, command-line
arguments, logs, or public documentation. Create separate private JSON files
outside the checkout, one per role, through your approved secret store.

Configuration requires an absolute path, a current-user-owned parent directory
with mode **0700**, and a current-user-owned regular file with mode **0600** and
one hard link. Every path component is opened without following symlinks.
The containing ancestor directories must be trusted; do not place the private
parent in a directory another user can rename. The loader caps configuration at
16 KiB and rejects duplicate/unknown keys, missing fields, noncanonical or zero
UUIDs, malformed wallet keys and unsafe URLs. HTTPS is required except literal
loopback HTTP. URL credentials, whitespace, query and fragment are rejected.
Robot audience is always explicit, including custom DDS deployments.

Keep `domain_id` and `installation_id` stable and equal for both workers and the
UI. Keep each worker's explicit `client_id` stable across restarts and distinct
from the other worker. These client IDs are installation identifiers, **not**
DDS compute/robot record IDs. In the UI, select the approved Domain and use
**Discover / refresh workers** to read activated workers through Fleet. Choose
an installation explicitly when several are found; no manual worker UUID entry
is required. Inspect public IDs in source details and verify receipt executors. Compute receives only its own registration and
wallet secrets; robot receives only its own registration secret and audience.
The file schema rejects credentials belonging to the other role.

Dedicated compute eligibility spans the organization's Domains. This adapter
additionally restricts every task to the configured Domain. Robot eligibility
requires its DDS-assigned Domain; startup checks that assignment before polling,
and every task checks it again through the task Domain. This code does not
reassign a robot. Unique installation capabilities route work; they do not prove
machine identity, online status, exclusive routing or application permission.

Validate without native imports, credentials being displayed, or network calls:

```sh
python3 -B core/examples/core-explorer/workers/run.py --check --config /absolute/private-workers/compute.json
python3 -B core/examples/core-explorer/workers/run.py --check --config /absolute/private-workers/robot.json
```

Exit 0 means structurally valid local configuration, not working credentials,
correct provider deployment, approved activation or online workers. Exit 2 is
configuration rejection; exit 1 is runtime failure. Logs contain only fixed event
strings, never IDs, input, credentials or exception bodies. Application code does
not enable SDK debug logging; review host-level diagnostics separately.

## Task contract and bounds

Use `/examples/compute-robot/{installation_id}/{role}/v1`, `mode: dedicated`,
`maxAttempts: 1`, one task at a time and no graph. Task metadata is exactly
`{"input_id":"<record-uuid>","run_id":"<installation-uuid>"}` and
`inputs_cids` is exactly that one input UUID. The upstream `run_id` field means
the stable installation ID here; it is not regenerated on restart. Unknown
metadata (including P2P instructions), wrong Domain, wrong capability or wrong
installation is rejected before obtaining `task.data()`.

Input streaming uses SDK `read_to(max_bytes=65536, max_chunk_bytes=16384)` plus
an independent sink bound. Compute rejects invalid UTF-8. Output is capped at
65,536 bytes, including Unicode uppercase expansion. Robot input may be binary;
its bounded JSON report describes those exact input bytes. Both write through
lease-authorized `task.data()` with per-task names
`sdk-{installation_id}-{role}-{task.id}`. Buffered named creation rejects existing
names; no replacement ID or multipart overwrite is used. This is not an atomic
exactly-once guarantee across provider races. A lost write response can leave an
artifact; never blindly repeat a submission to resolve uncertainty.

The handler has a 30-second execution deadline. Cancellation drains handler
cleanup and closes its data client. Startup/assignment and SDK requests also
have 30-second deadlines. Awaited cleanup may extend beyond those deadlines;
a timeout is not proof remote side effects stopped or credits were released.
The deterministic CPU work is bounded by the input size, with no subprocesses,
threads, shell commands or unbounded application retry loop. SDK authentication
renewal and bounded registration retries retain their existing semantics.

SIGINT/SIGTERM sets an explicit stop event. The runner cancels and drains the run
awaitable, awaits `tasks.close()`, then awaits `credential.close()`, including on
startup failure and runtime close failure. Repeated signals do not cancel that
cleanup. SDK lease loss/task cancellation follows the same handler cleanup path.

## Inactive user-service template and rollback

`core-explorer-worker@.service.example` is a portable template, not an installed
unit. Replace all `/REPLACE/absolute/...` paths with reviewed paths to the existing
virtual environment, checkout and separate private config directory. There is
no installer and no `[Install]` section. After separate activation approval, an
operator may prepare a user unit and explicitly start the `compute` and `robot`
instances. Do not enable unattended boot/login activation as part of validation.
Check the host's systemd/user-namespace and cgroup support for all restrictions;
unsupported hardening or an insufficient memory cap must fail for operator review.

The template caps CPU at 25%, memory at 256 MiB (192 MiB soft limit), tasks at 32,
file descriptors at 256 and journal rate at 10 messages/minute. Journal storage
retention remains host policy. Restart is limited to three starts in ten minutes,
with a 30-second delay, and configuration errors do not restart. SIGTERM allows
90 seconds for awaited shutdown; systemd may then kill the process, so this
hard limit cannot guarantee remote cleanup. Tune only after approved validation.

After activation, explicitly stop both instances with:

```sh
systemctl --user stop core-explorer-worker@compute.service core-explorer-worker@robot.service
```

Verify process exit and remote lease/presence expiry. Registration mutates
persistent capability sets and version/presence; record previous capabilities
before activation. Stopping does not restore those sets, erase jobs/artifacts,
reassign robots or guarantee released credits. Restoring capabilities through
supported registration, canceling jobs, deleting artifacts, disabling/removing
units or other shared-state cleanup requires its separately approved scope.
Retain receipts and uncertain-output evidence for reconciliation.

## Offline verification and limits

From the repository root:

```sh
python3 -B -m unittest discover -s core/examples/core-explorer/workers -p 'test_*.py' -v
```

The tests use the real upstream Python handlers with in-memory SDK boundary
objects, synthetic secrets and private temporary files. They test config-only
execution with an import/network guard; private permissions, ownership,
symlinks/hardlinks/FIFOs; task scope; streaming/input/output bounds; Unicode;
timeout/cancellation draining; robot assignment; and close ordering/failures.
They start no servers and do not import the native extension. They do not prove
native/provider compatibility, credential validity, real execution, deployed
robot support, cancellation credit accounting, or service hardening on a host.
Native builds, browser integration and live/service activation remain separate
validation gates. Existing backend constraints are documented in the
[jobs reference](../../../../docs/reference/jobs.md) and
[task guide](../../../../docs/how-to/run-compute-tasks.md).

The launcher disables core dumps and Linux dumpability before loading private
configuration or the native SDK, and sets `RUST_LOG=off`. Protection failure exits
with a fixed safe configuration error. The inert service template also sets
`LimitCORE=0` and `MemorySwapMax=0`.
