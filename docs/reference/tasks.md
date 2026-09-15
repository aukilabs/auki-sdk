# Task runtime reference

`AukiDmsTasks` manages DMS leases for native Rust and Python handlers.
For setup and examples, see [Run compute and robot tasks](../how-to/run-compute-tasks.md).
Web task execution is not exposed. Application runners, storage conventions,
and hardware control belong in the host application.

## Runtime and handler APIs

| API | Behavior |
| --- | --- |
| `AukiComputeCredential` | DDS node registration and wallet authentication |
| `AukiRobotCredential` | Robot registration, authentication, and assigned-Domain idle reads |
| `AukiDmsTasks::start` | Registers and authenticates; starts an assigned robot's configured peer without claiming work |
| `run` | Polls the registered capabilities serially and executes handlers |
| `run_once` | Attempts one capability; returns completed, no work, or busy |
| `close` | Cancels work and awaits handler, data, registration, and peer cleanup |
| `task.data()` | A data client authorized by the current lease |
| `task.peer()` | Optional peer view; Rust requires the `TaskPeerContext` trait |
| `task.progress(value)` | Replaces progress for the next heartbeat |
| `task.log_event(value)` | Appends an event for the next heartbeat |
| `task.set_failure(reason, details)` | Prepares a receipt to use if the handler fails |
| `task.access_token.get()` | Reads the current task Domain HTTP bearer |

Python handlers run on the application's asyncio loop with its context
variables. Progress, events, and failure setters are async in Python and
synchronous `Result` APIs on Rust `TaskContext`. Python results are `None` or
an `output_cids`/`meta` dictionary; Rust uses `TaskResult`. The runtime preserves
DMS `meta`, `inputs_cids`, `output_cids`, and completion metadata.

Each runtime permits one live lease across its clones. A compute credential
has one runtime owner. Concurrent managed `run()` calls are rejected. Python
`run_once(capability=...)` requires a capability when multiple handlers exist.
Handler and service failures stop the managed loop; DMS controls task retries.

Construction makes no network requests, though configuring a peer loads or
creates its identity file. `start`, `run`, and `run_once` establish machine
presence. Concurrent startup callers share one operation; cancelling a caller's
awaitable does not stop that operation. Await `close` to stop it.

## Defaults and limits

| Setting | Default or limit |
| --- | --- |
| Poll interval | 1 second between rounds |
| Heartbeat interval | 30 seconds |
| Request timeout | 30 seconds |
| Registration interval | 120 seconds |
| Transient registration failure | At most three attempts |
| Progress | 64 KiB serialized JSON |
| Python handler result | 64 KiB serialized JSON |
| Event queue | 1,024 events and 64 KiB serialized JSON |
| Failure reason | Nonblank, at most 4,096 UTF-8 bytes |
| Failure details | 64 KiB serialized JSON |
| DMS response envelope | 2 MiB |

Rust configures runtime intervals with `TasksConfig` and machine settings with
`ComputeConfig` or `RobotConfig`. Python exposes corresponding constructor
keywords. Data operations use the [Domain data limits](domain-data.md#limits).

## Events and receipts

`log_event` preserves enqueue order, including across cloned Rust contexts.
A full queue rejects new events without dropping accepted ones. Only events
acknowledged by DMS leave the queue. Progress is independent and keeps only
its latest value.

Managed execution waits for an in-flight heartbeat, then flushes final progress
and events before reporting completion or failure. A failed heartbeat ends the
lease. An unknown network outcome is not retried; a confirmed HTTP 401 allows
one authenticated replay. Delivery is not exactly-once, and DMS applies its
configured event-retention limit.

`set_failure` prepares a reason and JSON details without sending a request or
ending execution. Details can include artifact metadata. A later call replaces
the receipt; a successful handler ignores it. To report it, a Python handler
raises and a Rust handler returns `Err(TaskError::Handler)` after cleanup.
Without a prepared receipt, failure uses a generic reason. Python exceptions
are re-raised locally; their text is not automatically sent to DMS.

Cancellation or authority loss skips completion and failure receipts, including
prepared receipts. A receipt already sent cannot be retracted, and its remote
outcome may be unknown. Include only text and metadata intended for DMS, without
credentials or private exception details.

## Current task token

`task.access_token.get()` synchronously reads the current task Domain bearer.
The handle shares lease rotation and revocation; it does not refresh tokens
independently. Read it before each HTTP request rather than storing a token copy.
It is neither a machine credential nor a P2P token.

Reads fail after lease expiry, cancellation, runtime shutdown, or task completion
or failure. Rust returns a redacted, zeroizing `SecretString`; call
`expose_secret()` only when handing it to your HTTP client. Do not log tokens.
Previously returned copies may remain valid remotely until expiry.

For SDK data operations, use `task.data()`. Its 401 handling asks the existing
heartbeat owner for renewal, and its transfers participate in lease cleanup.

## Authority and peer lifetime

Task data grants are checked for Domain, issuer, server audience, and expiry
against the authenticated DMS envelope. Domain Servers verify signatures and
permissions. Retained task data clients lose access when the lease ends.

Idle robot data uses DDS `/internal/v1/auth/robot/domain-token` with exactly
`domain:r`. Writes, streaming uploads, and deletes fail locally. Assignment and
machine identity stay fixed for the credential's lifetime; DDS checks the
persisted assignment on each exchange. An unassigned robot maintains presence
without claiming work. Assignment changes require shutdown and new credentials.

| Peer | Credential source | Lifetime |
| --- | --- | --- |
| Compute | DMS lease and heartbeat | One task; closes on completion, failure, or cancellation |
| Robot | DDS assigned-Domain P2P exchange | Runtime; remains connected between tasks |

The SDK binds the Peer ID during machine login before claiming work. Signed
peer credentials must match the machine type, Peer ID, Domain, issuer, audience,
expiry, and required `domain-data:r` scope. Keys and credentials rotate through
the peer authority supervisor. A robot's runtime renews DDS peer authority while
idle or busy, independently of DMS task heartbeats.

Python configures P2P with `peer_identity_file` and optional `peer_config`.
Rust uses `AukiTaskPeerConfig`, its `identity_proof()` in the machine config,
and `AukiDmsTasks::new_with_peer`. Defaults and routes follow
[`AukiPeerConfig`](networking.md#configuration).

`task.peer()` exposes a protocol view without authority controls. The runtime
owns shutdown; Python rejects `shutdown()` on the view. Retained compute views
stop working at task end; robot views remain usable between tasks. `tasks.peer()`
exposes only an assigned robot's configured peer after startup. Applications
own protocol endpoint lifetimes and authorization for their operations.

Ordinary managed execution requires valid peer authority when P2P is configured.
Missing grants fail startup. Native hosts can explicitly accept HTTP-only compute
leases through `execute_with_optional_peer`, described below. Invalid supplied
grants still fail. P2P carries application data; DMS owns scheduling and leases.

## Cancellation and errors

Cancelling a Python run awaitable requests native cancellation. `close()` waits
for the actual handler's `finally` blocks and resource cleanup. Call it from
the host, outside the handler it awaits, and keep the asyncio loop alive until
it returns. Close is repeatable, including after a previous close was cancelled.
Close the machine credential after its runtime and any idle data clients.

Rust handlers must observe cancellation and finish cleanup before returning.
Threads, subprocesses, CPU work, and hardware need a host stop mechanism. The
runtime keeps its execution slot until cleanup completes. Await cancellation
and close; dropping a Rust future cannot run async cleanup.

Lease loss, DMS cancellation, invalid credential rotation, and terminal
registration failure stop local authority and cancel the handler. Robot
authority or assignment loss also stops its persistent peer. Runtime shutdown
awaits peer transport, discovery, and relay cleanup.

Python `TaskRuntimeError.kind` distinguishes configuration, authentication,
authority, lease loss, cancellation, busy/closed state, HTTP, service, data setup,
and cleanup errors. HTTP errors retain `status`; data operations use
`DomainDataError`. Peer cleanup failures are retained for `tasks.close()` even
if the run awaitable was cancelled. Rust callers must handle its `Result`.

Authentication renewal is serialized. Only HTTP 401 permits one authenticated
replay; 403 remains a denial. Other claim, completion, or failure errors can
leave the remote outcome unknown. DMS retries can repeat application side effects;
use task or operation IDs where duplicate execution must be prevented.

## Native runner adapters

Use `claim_any()` to let DMS choose across registered capabilities, followed by
`lease.execute(handler, cancellation)` for managed heartbeats, events, reporting,
and cleanup. The host can stop claiming with one cancellation token and reserve
another to interrupt active execution. Release the lease before closing the runtime.

`execute_with_optional_peer()` accepts compute leases with no P2P grant as
HTTP-only work. Partial, expired, or invalid grants still fail. Robot peers keep
their runtime lifetime. A runner that needs P2P must check for a peer before work.

`task.credential.lease_snapshot()` supplies current lease metadata, including
initial-heartbeat metadata and the Domain bearer, for existing Rust runner
interfaces. P2P credentials are excluded. Do not log the snapshot; use
`task.access_token.get()` for later HTTP requests.
`AukiTaskPeer::subscribe_status()` lets hosts observe peer authority and relay
readiness to pause claims or stop application work.

For a manual loop, `claim()` returns a `TaskLease` with `heartbeat()`,
`complete()`, and `fail()` operations. The host owns heartbeat timing. With P2P
configured, call `start_peer()` after the initial heartbeat. Await `lease.close()`
to cancel local work without reporting a result. Dropping it revokes local data
access and starts best-effort peer cleanup; await close for orderly shutdown.
`AukiDmsTasks::from_client` accepts an authenticated `DmsClient`; the host must
stop the runtime when that client's external authority ends.

`DmsClient::lease_by_capability` sends its capability filter. Callers that need
DMS to choose across authorized capabilities must use `lease_any()` or the
runtime's `claim_any()` when updating an older SDK dependency.

## Service requirements

Use provisioned credentials and aligned DDS, DMS, and discovery endpoints.
Compute authentication uses DDS SIWE and node registration routes. Robots require
`DDS_ROBOT_WORKERS_ENABLED` in DDS, `ROBOT_WORKERS_ENABLED` in DMS, and matching
exclusive `DDS_ROBOT_AUDIENCE` configuration.

Compute P2P for custom capabilities requires DMS support for issuing peer grants
across registered capabilities. The provider contracts used by the SDK include
[DDS robot support at b27c0804](https://github.com/aukilabs/domain-service/commit/b27c0804)
and [DMS compute P2P at 8088111](https://github.com/aukilabs/domain-manager-service/commit/8088111).
Older DMS revisions issue compute P2P grants only for selected built-in capabilities.

Events and failure receipts use DMS heartbeat `events` and failure `reason` /
`details` fields, as in [DMS 004688cb](https://github.com/aukilabs/domain-manager-service/commit/004688cb).
Source support does not establish deployed support: verify the environment's
provider versions, robot flags, and audience before running a machine.
