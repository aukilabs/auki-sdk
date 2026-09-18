# Core Explorer Chat addon

Chat is an optional, example-owned addon, not a stable SDK protocol. The root
crate's default feature set is empty. The combined Web and Python example
modules enable `chat`; standalone native and Swift Echo builds do not. Echo's
wire format and API remain unchanged. Peer handles must come from the same
compiled module as Chat.

Protocol `/example/core-explorer-chat/1.0.0` uses a four-byte big-endian JSON
length and at most 16,384 total frame bytes (including that prefix), checked
before allocation. JSON has a `type` discriminator: `pending` and `paired`
carry `session_id`; `message` adds `id` and `text`; `ack` adds `id`. UUIDs are
validated, unknown fields rejected, and all frames belong to the live session.
Text must contain 1–2,048 UTF-8 bytes. Unknown/replayed ACKs close the stream.
No protocol compatibility with Echo or other Chat versions is implied. Deploy
both matching example adapters together; no backend contract changes are made.

The server creates the public correlation UUID and reports the authenticated
transport Peer ID. The operator must approve that exact pending pair. Any byte
received before approval terminates the stream without reading a chat payload.
There is no automatic trust, AI invocation, command execution, or task dispatch.
`send` completion means written, and an `ack` means accepted by the remote
protocol's bounded event queue, never human-read or AI execution. Do not blindly
retry after an error: a partial write may have reached the remote peer.

At most four sessions coexist. Pending approval expires after five minutes;
each stream expires one hour after the server starts handling it, including
pending time. The host itself has no expiry. Queues hold 128 entries; overload
is an explicit error/closed event channel. To bound replay tracking as well as
queues, a session permits at most 128 messages total across both directions,
with one shared ID namespace; reconnect and approve a new session to continue.
Session UUIDs are not reused on reconnect.
Opening, hello, body I/O, writes, and stream close have five-second deadlines.
Idle/header reads are bounded by the session lifetime rather than Echo's timeout.

The combined Web module exports `AukiChatConnection.connect(peer, peerId,
wssRoute)`, `sessionId`, `nextEvent()`, `send(id,text)`, `close()`, and generated
`free()`. It registers no inbound handler. `nextEvent()` returns JSON events
`paired`, `message` (`id`,`text`), or `ack` (`id`). Close retains and replays
stream cleanup errors; await it before closing the peer/session, then free the
Wasm wrapper.

The combined Python module exports `AukiChatHost.mount(peer)`, `protocol`,
`next_event()`, `approve(session_id,peer_id)`, `send(session_id,id,text)`, and
`close()`. Events include `pending`, `paired`, `message`, `ack`, and `closed`,
all with `session_id` and authenticated `peer_id`. Cleanup is detached and
replayable like EchoOwner. Host close attempts every live stream cleanup and
registration cleanup, and reports retained stream cleanup failures even after
those sessions have disconnected. Close both Chat and Echo before the shared
peer and session. Operator inbox/outbox persistence is owned by the separate chat-host
application, not this protocol.

Regression tests live in `src/chat.rs`, `src/chat_tests.rs` (bounded in-memory
duplex driver tests), and `python/src/chat.rs`. Native protocol tests do not
substitute for Python/Wasm compilation, real Wasm-to-native local acceptance,
or UI lifecycle/mobile checks. Those are separate parent validation gates.
