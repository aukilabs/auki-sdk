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
