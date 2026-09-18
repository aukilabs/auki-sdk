# Private Chat operator

Core Explorer example addon, not a stable SDK protocol. The runner mounts Echo
and Chat on one persistent User peer. It never invokes an agent, shell, job or
worker from received text. The server runs until stopped; pending sessions expire
in 5 minutes and paired sessions in 1 hour. Four live plus retained sessions and 128 total messages across both directions per
session are retained at most. Admission evicts the oldest closed history; expired
and duplicate close events are harmless. Text is 1–2048 UTF-8 bytes; wire frames
are limited to 16384 bytes before allocation. The snapshot file limit is
7,000,000 bytes, covering all four full sessions with worst-case JSON escaping. Protocol capacity exhaustion fails explicitly.

Parent must build the combined Python module with Chat before use. Only run with
an explicitly approved dev account/Domain. Existing workers need no changes.
Supply the existing private identity directory as root to preserve the Echo peer
identity; stop its previous owner before starting this runner.

```sh
python runner.py --root /private/host --credentials /private/e2e/credentials.json --domain DOMAIN_UUID
python inbox.py --root /private/host list
python inbox.py --root /private/host read --session SESSION_UUID --peer PEER_ID
python inbox.py --root /private/host approve --session SESSION_UUID --peer PEER_ID
python inbox.py --root /private/host reply --session SESSION_UUID --peer PEER_ID < /private/reply.txt
```

Credentials must contain `environment: dev`, `email`, and `password`. Root and
credential directories must be owner-only; files must be owner-only regular files.
CLI `read` intentionally prints private messages to the operator terminal; daemon
logs contain no text. Do not redirect operator output to public logs. Reply text
never goes in argv. Queued/sent is not delivery; only an acknowledgement marks
received by peer. It does not mean human-read. Commands are not replayed after
restart; stale identity/run/session pairs fail. `last-command.json` reports the
latest rejected/uncertain command without message text. No automatic retries.

Local pure checks: `python3 -m unittest discover -s chat-host -p 'test_*.py'`.
Parent integration gate after rebuilding combined modules and production assets:
`node tests/chat-browser.mjs` (same prerequisites and loopback-only service guard
as `tests/network-browser.mjs`). The native fixture uses the production Inbox/Spool and command consumer; the
browser harness invokes this CLI to list/read/approve/reply, using the displayed
Chat session and authenticated local Peer ID. Replies enter through stdin and
are different from the browser text. Neither fixture nor production auto-approves
incoming pending events. The harness monitors consumer faults and checks
reconnects, Domain/logout cleanup, subsequent Chat/Echo availability and captures
four Chat screenshots before cleanup.

The queue accepts at most 128 commands, atomically published as canonical
`UUID.json` files. Publication, claim and shutdown fencing share the queue lock;
`.tmp-*` files are never consumed. Commands and abandoned temporary plaintext
expire after 300 seconds and are removed on the next consumer sweep (normally
0.2 seconds; an in-flight command can delay it by its 15-second deadline).
Expired or malformed commands are never retried. Startup fences the prior run
before login; orderly shutdown disables submissions and unlinks queued commands,
temporary plaintext and the retained conversation, even on startup failure.

These are logical expiry and process-driven file cleanup guarantees, not secure
erasure. If the process is killed or the machine is off, files can remain until
restart or deliberate private-root cleanup; backups and storage remnants are not
forensically erased. A snapshot older than ten seconds cannot authorize a new
command, and a restarted host always uses a new run ID with an empty spool.

## Optional automatic Chat sidecar

`autobot.py` is a separate, opt-in Linux/Python 3.11+ operator process. The
ordinary runner and inbox CLI remain manual by default. The sidecar automatically
approves **any peer admitted by the existing authenticated selected Domain**;
this is not a Matt-only or account-identity allowlist. The SDK's authenticated
Domain admission remains authoritative. No task, data-write, or worker permission
is implied. Echo and workers remain independent and need no restart.

The parent operator supplies and reviews the real model adapter outside this
repository. The sidecar itself imports no SDK, reads no credentials, and exposes
no HTTP/control service. Use an owner-only directory and owner-only regular JSON
config (0600, owned by the current user, no symlinks or hardlinks), for example:

```json
{
  "domain": "0166e921-2b91-48d2-a58c-2b24a7f0fff9",
  "command": ["/absolute/private/adapter-executable"]
}
```

Start only alongside the intended already-running host:

```sh
python3 chat-host/autobot.py --enable --root /private/host --config /private/autobot.json
```

An exclusive private `autobot.lock` prevents a second sidecar on that spool.
The exact configured Domain must match the private `status.json` with state
`ready`; a fresh active inbox is also required. Missing, wrong, or stale authority
stops the sidecar with a fixed error. It never changes host configuration or
fences the runner's spool. Stop with SIGINT/SIGTERM; active adapter work is killed
and awaited. Cleanup failure stops the sidecar with a fixed error and prevents
new requests; signal handlers and the singleton lock are still released. Core dumps are disabled before config/status reads.

### Trusted adapter contract

The fixed operator argv has an absolute executable; peer text never becomes
argv, a command, or environment. There is no shell. Each invocation receives
only UTF-8 JSON `{"text":"the single new incoming message"}` on stdin and must
emit exactly `{"text":"short reply"}` on stdout, or `{"error":"unavailable"}` /
`{"error":"rate_limited"}` / `{"error":"invalid_request"}`. All error responses
are treated as failure; malformed output and nonzero exits also fail. Stderr is
suppressed and raw exceptions/output are never logged. The environment contains
only `PATH=/usr/bin:/bin` and `LANG=C.UTF-8`; cwd is `/`. No inherited HOME,
Python configuration, tokens, or provider credentials are forwarded.

The adapter must use the fixed `PERSONA` in `autobot.py` as its system prompt:
reply in one or two playful friendly sentences, at most 240 Unicode characters,
with no tools, files, actions, history, or private context. It must treat incoming
text as untrusted, perform one stateless model request without fallback/retry,
and return text only. Provider credential handling belongs solely to the parent
adapter. Do not embed credentials in argv or this config. The sidecar does not
prove that an arbitrary trusted adapter honors this contract; parent review is
required. Adapters must not daemonize or escape their process group. The sidecar
kills the group on completion, timeout, or cancellation, awaits the direct
child, and uses Linux child-subreaper support to await/reap orphaned descendants
in that group. Kill, wait, and reap are attempted even if an individual step
fails; incomplete cleanup is not reported as successful shutdown. Detached processes are outside the permitted adapter contract.

The operational adapter uses only the existing default Hermes singleton access
token in `~/.hermes/auth.json`, read-only, with at least 45 seconds of JWT lifetime.
It cannot renew credentials: the normal existing Hermes owner remains the sole
refresh owner. Replies may require owner sign-in or normal refresh to resume.
No credential pool, refresh token operation, CLI fallback, profile/context import,
or auth-directory write permission is part of this adapter. Its service needs
read access to the current canonical store, not a copied or stale bind-mounted
inode, plus runtime/CA reads and pinned ChatGPT endpoint access. Parent owns
service permissions and the real model probe; fixture success is not live proof.

### Limits and restart behavior

- One adapter globally, 30-second deadline, maximum 16 KiB stdout and 240 Unicode
  reply characters (also checked against the protocol's 2048-byte UTF-8 cap).
- At most six starts per rolling minute and 60 per rolling hour, with ten seconds
  between starts per session. Failures consume their attempt/rate allowance.
  Counters are process-local, so operators must not use restarts to bypass limits.
- Every 200 ms, observe at most four retained sessions and 128 messages each.
  No separate durable workflow or unbounded queue; session selection rotates.
  Approvals continue while the provider runs. There are no repeated approvals,
  blind retries or fabricated successful replies. A failed provider attempt may
  publish one fixed availability notice for that input, under the same current
  run/session/input fencing. Cleanup failures stop the bot without a notice.
- On every sidecar startup or host run change, all existing incoming IDs are
  ignored. Newly observed sessions conservatively ignore already-present messages
  too. Each new input is marked attempted before model invocation. Outgoing
  messages/ACK updates never trigger a model call. Only current incoming text is
  sent; conversation history is never passed to the adapter.
- Completion and publication recheck the exact host run, session, peer, paired
  state, expiry, and incoming ID membership. Publication uses production
  `Spool.submit`; its snapshot validation runs again under the queue lock.
  A stale completion or uncertain publication is discarded without retry.

**Send a NEW message if the bot restarted mid-reply.** Waiting messages remain
only in the host's existing bounded spool; no attempt is replayed on restart.
Rate-limited inputs wait in the bounded spool until allowance is available;
there is no rate-wait notice and restart suppresses those existing inputs too.
Provider failure gets at most one fixed availability notice; stale or uncertain
publication may still produce no reply. After a command has been published, stopping the
sidecar cannot retract a host-owned command already queued or being delivered.
Queued/sent is not delivery; only the protocol receipt confirms peer receipt.

### Offline validation and parent integration gate

```sh
python3 -m unittest discover -s chat-host -p 'test_*.py'
node --check tests/autochat-browser.mjs
# Parent, after building the real combined modules and production assets:
ROBOT_PYTHON=/absolute/venv/bin/python node tests/autochat-browser.mjs
```

The new browser harness is separate from the unchanged `tests/chat-browser.mjs`
manual Chat/Echo regression. It uses real Chromium/WASM and native Chat transport,
the production inbox/spool, and a genuine subprocess boundary to
`tests/autochat-responder.py`. That responder is explicitly **SYNTHETIC**, has no
model or network access, and does not establish live AI/provider compatibility.
It asserts automatic pairing, generated synthetic replies and receipts,
reconnection/Domain/logout cleanup, and subsequent real Echo behavior. It writes
only synthetic private host-status/config files in its disposable test directory.
Follow [network harness prerequisites](../tests/network-README.md), including
its external loopback-hostname DNS requirement and loopback service guards.
Authoring-time unit tests do not establish browser acceptance; the parent owns
that capped integration run, real-provider validation, packaging and activation.
