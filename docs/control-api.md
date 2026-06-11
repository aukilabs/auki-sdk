# Cross-app HTTP Control API — v1

The Auki SDK's operator-control surface. Daemons that produce SDK sessions (BoosterApp, Sentinel, future) implement this API so any UI — primarily Park — can drive any of them through a single contract.

This is **not** part of the data protocol. Data products flow through the SDK's on-disk format (registries, logs, segments). The Control API is a runtime, operator-facing concern: list and manage sensor logs (live and historical), peek at the latest captured frame, request a clean shutdown.

It is also **not** an identity surface. Peer identity (`auki_network::ParticipantInfo`) is exchanged exclusively over the SDK's libp2p `/auki/info/0.0.1` protocol, gated by cluster membership — see [Identity](#identity-not-served-here) below.

> **Status — HTTP frozen at SDK release v0.0.23.** This is the terminal HTTP revision of the Control API. Subsequent control-plane evolution lands as libp2p protocols (`/auki/control/sensor_logs/1.0.0`, …) using the length-prefixed framing pattern of the existing `/auki/` peer protocols. The data model in this document (unified `sensor_logs`, cross-session listing, `session_id` everywhere) is transport-neutral; libp2p protocols will adapt the same shapes once the in-process surface (`auki-session` / `auki-session-py`, shipped in #216 / #224) stabilizes. No new HTTP endpoints will be added; existing endpoints stay maintained — with one removal: `GET /api/info` is gone from the contract entirely ([#293](https://github.com/aukilabs/auki-sdk/issues/293)), because identity is libp2p-only.

## Conformance

A daemon is **Auki Control API v1 conformant** when it:

1. Implements every endpoint below with the exact request/response shapes specified.
2. Returns a JSON `{"error": "<message>"}` body for any non-success status code outside the response shapes specified per endpoint.
3. (Optional but recommended) Advertises itself via mDNS per the convention below.

Consumers (Park, etc.) MAY auto-discover daemons via mDNS and MUST also support a manual fallback (`--daemon <url>` or equivalent).

---

## Endpoints

All endpoints live under `/api/`. Daemons bind `0.0.0.0:<port>` — no authentication, trusted-LAN assumption (see [Security model](#security-model) below).

### Identity (not served here)

There is **no HTTP identity endpoint** in this contract. Peer identity — `auki_network::ParticipantInfo`: `app`, `name`, `session_id`, `session_clock_id` + `session_clock_hash`, `session_now_ns`, `cluster_joined_at_ns`, `peer_id`, `app_instance`, `is_manager`, `manager_peer_id` — is exchanged exclusively over the SDK's libp2p `/auki/info/0.0.1` protocol, gated by cluster membership and the runtime allow-list ([#293](https://github.com/aukilabs/auki-sdk/issues/293)).

The earlier `GET /api/info` endpoint is removed from this spec: serving `ParticipantInfo` over unauthenticated HTTP leaked identity, session, and Manager metadata to any LAN client without cluster admission. An app MAY still expose a local identity endpoint for its own UI (Park's browser-facing `/api/info` is one) — such endpoints are app-local, operator-facing/debug surfaces, never a cross-app contract, and never a source of cluster peer identity.

**No canonical clock.** The SDK does not assume UTC, monotonic, or any other specific clock as canonical for the API. Every timestamp is paired with an explicit clock identity (e.g. a log's `clock_id` + `clock_hash`); cross-clock conversion is what the [TimeTransform Log](../crates/auki-time/README.md) and `convert_time` exist for. Apps that treat UTC as canonical do so by *convention* — they configure a TimeTransform between their session clock and a UTC clock and consumers walk it via `convert_time`.

### `GET /api/registries/sensors/<sensor_id>/<sensor_hash>`

Return a Sensor Registry entry by its content-addressed hash. The response body is the on-disk JSON at `<app_root>/registries/sensors/<sensor_id>/<sensor_hash>.json` served verbatim — the SDK's [`auki-registry`](../crates/auki-registry/README.md) crate owns the schema; this endpoint is a thin file-server.

Hash-keyed entries are immutable: once a `(sensor_id, sensor_hash)` pair is written, it never changes. Consumers cache aggressively.

**Request.** No body.

**Response.**

- `200 OK`, `application/json`, with `Cache-Control: public, max-age=31536000, immutable` — the registry entry. Body shape is whatever [`auki-registry`](../crates/auki-registry/README.md) defines for sensors (e.g. `data_type`, `width`, `height`, `frame_rate_hz`, `pixel_format`, intrinsics).
- `404 Not Found`, `application/json`: `{ "error": "no such sensor entry" }` — `sensor_id` exists but the requested `sensor_hash` is not on disk, or `sensor_id` itself is unknown.

### `GET /api/registries/clocks/<clock_id>/<clock_hash>`

Return a Clock Registry entry by its content-addressed hash. Same shape, semantics, and caching guarantees as the sensors endpoint above; body is the on-disk JSON at `<app_root>/registries/clocks/<clock_id>/<clock_hash>.json` served verbatim per the [`auki-registry`](../crates/auki-registry/README.md) clock-entry schema (e.g. `kind`, `epoch`, `scope`).

**Request.** No body.

**Response.**

- `200 OK`, `application/json`, with `Cache-Control: public, max-age=31536000, immutable` — the registry entry.
- `404 Not Found`, `application/json`: `{ "error": "no such clock entry" }` — `clock_id` exists but the requested `clock_hash` is not on disk, or `clock_id` itself is unknown.

A future Frame Registry endpoint will follow the same shape (`/api/registries/frames/<frame_id>/<frame_hash>`); not yet specified.

### `GET /api/preview/latest.jpg`

Most recent frame captured by the daemon, encoded as JPEG. Poll-based — see [v1 design choices](#v1-design-choices-deliberate) for why no streaming.

**Request.** No body.

**Response.**

- `200 OK`, `image/jpeg` — the latest frame.
- `503 Service Unavailable` — no frame captured yet (daemon just started, no source data).

### `POST /api/sensor_logs`

Open a new sensor log in the **current session**. A sensor log is a single `auki-logs::Log<T>` instance writing one sensor's stream to disk under `<session>/sensorlogs/<sensor_log_id>/`. Buffers, intent recordings, and time-bounded captures are all sensor logs — they differ only in the values of `retention_ns` and `duration_ns`.

**Request.** `application/json`:

```json
{
  "sensor_id": "K1-AABBCCDDEEFF/head_left_cam",
  "sensor_hash": "abc123...",
  "retention_ns": 30000000000,
  "duration_ns": 0
}
```

| Field | Type | Notes |
| --- | --- | --- |
| `sensor_id` | string | The sensor to stream from. Must be a sensor the daemon has bound for this session. Resolves via [`GET /api/registries/sensors/<sensor_id>/<sensor_hash>`](#get-apiregistriessensorssensor_idsensor_hash). |
| `sensor_hash` | string | Content-addressed hash pinning the schema version the log is opened against. Required even though it is locally redundant within a single session — useful for cross-session data transfers and defensive against schema drift. If the daemon's live binding for `sensor_id` has a different hash, the request is rejected (see error responses). |
| `retention_ns` | integer | Backward window kept on disk in nanoseconds. Segments older than this aged-out window are evicted on each `append`. `0` disables eviction (keep everything). Same field as `auki_logs::manifest::retention_ns`. |
| `duration_ns` | integer | Forward cap in nanoseconds. The daemon auto-stops the log after this much time has elapsed on the log's clock since `started_at_ns`. `0` disables the cap (run indefinitely). |

The four corners of the `(retention_ns, duration_ns)` plane:

| `retention_ns` | `duration_ns` | Use case |
| --- | --- | --- |
| `0` | `0` | Unbounded capture — runs forever, keeps everything. |
| `30000000000` | `0` | Rolling pre-roll buffer — runs forever, evicts segments older than 30s. |
| `0` | `60000000000` | Time-boxed capture — runs for 60s, keeps everything. |
| `30000000000` | `60000000000` | Time-boxed buffer — runs for 60s, keeps only the last 30s on disk. |

**Response.**

- `201 Created`, `application/json`: `{ "sensor_log_id": "<uuid>" }`.
- `400 Bad Request`, `application/json`: `{ "error": "<message>" }` — malformed body, negative `retention_ns` / `duration_ns`, unknown `sensor_id`.
- `409 Conflict`, `application/json`: `{ "error": "sensor_hash mismatch" }` — `sensor_id` is bound but at a different `sensor_hash` than the request specified. The daemon does not silently accept the live binding's hash — schema drift is the request's job to resolve.

The response shape on `201` is `{"sensor_log_id"}` only. Clients that want the full per-log fields (clock identifiers, `started_at_ns`, etc.) follow up with [`GET /api/sensor_logs?session_id=current`](#get-apisensor_logs) or [`GET /api/sensor_logs/<id>`](#get-apisensor_logsid)-style filtering.

### `GET /api/sensor_logs`

List sensor logs the daemon can see. **Spans every session on disk by default** — both the live session and any prior sessions whose logs the daemon's app-root contains. With no query parameters, returns every sensor log the daemon can enumerate.

**Request.** No body.

**Query parameters** (all optional; multiple compose as AND):

| Param | Value | Effect |
| --- | --- | --- |
| `session_id` | `<uuid>` or the literal `current` | Restrict to one session. `current` means the daemon's live session — the same `session_id` the daemon's `ParticipantInfo` carries over libp2p `/auki/info/0.0.1`. |
| `sensor_id` | `<id>` | Restrict to one sensor — useful for "find every log this camera ever wrote." |
| `sensor_hash` | `<hash>` | Restrict to one schema version of a sensor. Cross-session by design: "find every log written against this exact camera schema." |
| `clock_id` | `<id>` | Restrict to logs whose per-frame timestamps live on this clock. |
| `started_after` | `<ns>` | Restrict to logs with `started_at_ns >= started_after`. Interpreted on each log's own `clock_id` — see the clock-interpretation note below. |
| `started_before` | `<ns>` | Restrict to logs with `started_at_ns < started_before`. Same clock-interpretation rule as `started_after`. |

**Response.** `200 OK`, `application/json`:

```json
{
  "sensor_logs": [
    {
      "sensor_log_id": "0b3e...c41a",
      "session_id": "abc-123-...",
      "sensor_id": "K1-AABBCCDDEEFF/head_left_cam",
      "sensor_hash": "abc123...",
      "clock_id": "K1-AABBCCDDEEFF/utc",
      "clock_hash": "def456...",
      "retention_ns": 30000000000,
      "duration_ns": 0,
      "started_at_ns": 1745000000000000000,
      "stopped_at_ns": null
    },
    {
      "sensor_log_id": "1f7d...e88b",
      "session_id": "abc-123-...",
      "sensor_id": "K1-AABBCCDDEEFF/head_left_cam",
      "sensor_hash": "abc123...",
      "clock_id": "K1-AABBCCDDEEFF/utc",
      "clock_hash": "def456...",
      "retention_ns": 0,
      "duration_ns": 60000000000,
      "started_at_ns": 1745000045000000000,
      "stopped_at_ns": 1745000105000000000
    }
  ]
}
```

| Field | Type | Notes |
| --- | --- | --- |
| `sensor_logs` | array | All sensor logs matching the filter, ordered by `started_at_ns` ascending. |
| `sensor_logs[].sensor_log_id` | string | Daemon-assigned identifier; opaque to consumers. Unique within `session_id`; cross-session collisions are not guaranteed to be absent (consumers key by `(session_id, sensor_log_id)` if they need a global handle). |
| `sensor_logs[].session_id` | string | The session this log belongs to. May or may not match the daemon's live session — consumers filter by `session_id=current` when only the live session is wanted. |
| `sensor_logs[].sensor_id` | string | The sensor this log streams from. Resolves via [`GET /api/registries/sensors/<sensor_id>/<sensor_hash>`](#get-apiregistriessensorssensor_idsensor_hash). |
| `sensor_logs[].sensor_hash` | string | Content-addressed hash pinning the exact sensor entry the log was opened against. The hash IS the version — don't substitute. |
| `sensor_logs[].clock_id` | string | The clock used for the log's per-frame timestamps. Resolves via [`GET /api/registries/clocks/<clock_id>/<clock_hash>`](#get-apiregistriesclocksclock_idclock_hash). |
| `sensor_logs[].clock_hash` | string | Content-addressed hash pinning the exact clock entry. Same hash-is-version rule. |
| `sensor_logs[].retention_ns` | integer | Configured retention window. `0` = no eviction. Mutable on live logs via [`PATCH /api/sensor_logs/<id>`](#patch-apisensor_logsid). |
| `sensor_logs[].duration_ns` | integer | Configured forward cap. `0` = no cap. Mutable on live logs via [`PATCH /api/sensor_logs/<id>`](#patch-apisensor_logsid). |
| `sensor_logs[].started_at_ns` | integer | ns on the clock identified by `clock_id`. Set when the log was opened. The SDK does not assume UTC — see the [no-canonical-clock note](#identity-not-served-here). |
| `sensor_logs[].stopped_at_ns` | integer \| null | Same clock as `started_at_ns`. Set the moment the log stopped — either by [`DELETE /api/sensor_logs/<id>`](#delete-apisensor_logsid), by hitting its `duration_ns` cap, or by daemon shutdown. `null` only when the log is currently live in the daemon's current session. Logs from prior sessions always have `stopped_at_ns` set; if a daemon crashed mid-session the recovered log carries the last successfully-written timestamp as `stopped_at_ns` (see [`auki-session`](../crates/auki-session/README.md) for crash-recovery semantics). |

**Liveness and extent.** The "is this log still being written" signal is `stopped_at_ns == null` paired with `session_id == <live session>`. Consumers can compute extent from `(stopped_at_ns or session_now_ns) − started_at_ns`; the daemon does not pre-compute and ship a duration field for that — `duration_ns` is the configured *cap*, not the elapsed extent. (The pre-v0.0.23 spec defined `duration_ns` as elapsed extent; that interpretation is gone — see [the changelog entry that introduced this surface](../changelog.md).)

**`started_after` / `started_before` clock interpretation.** Each log's `started_at_ns` is on its own `clock_id`. Filter values are compared per-log, so a `started_after` value is meaningful when the daemon's logs share a clock or when the consumer knows which clock to address. BoosterApp v1 writes every log under a single `CLOCK_REALTIME`-backed clock, so its filter values are wall-clock nanoseconds; daemons running heterogeneous clocks document their filter semantics in their daemon-level docs.

**Cross-session enumeration.** The default (no filter) lists every sensor log the daemon's app-root contains, including logs from prior sessions whose `auki-layout` directories the daemon can read. Daemons that have not yet enumerated their on-disk sessions at startup MAY return only the live session's logs; this is a daemon-implementation latitude, not a spec relaxation — operators should expect cross-session listing once the daemon is steady-state. The daemon must be running for any request to succeed (it's an HTTP server); a no-live-session "browsing-only" daemon mode is out of scope for v1.

### `PATCH /api/sensor_logs/<id>`

Mutate a live log's configuration. Only `retention_ns` and `duration_ns` are mutable; identity fields (`sensor_id`, `sensor_hash`, `clock_id`, `clock_hash`, `session_id`) are immutable — changing any of them is semantically a different log.

**Request.** `application/json` — at least one of:

```json
{ "retention_ns": 60000000000, "duration_ns": 0 }
```

| Field | Type | Notes |
| --- | --- | --- |
| `retention_ns` | integer (optional) | New retention window. `0` disables eviction. Backed by `auki_logs::Log<T>::set_retention` — manifest is rewritten atomically; eviction is not retroactive (runs only on next `append`). |
| `duration_ns` | integer (optional) | New forward cap. `0` disables the cap. The daemon may shorten or extend the cap on the same elapsed clock. Setting `duration_ns` to a value already exceeded by the elapsed runtime stops the log immediately on the next tick. |

**Response.**

- `200 OK`, `application/json` — echoes the post-PATCH values:
  ```json
  { "retention_ns": 60000000000, "duration_ns": 0 }
  ```
- `400 Bad Request`, `application/json`: `{ "error": "<message>" }` — malformed body, negative values, attempting to mutate any field outside `retention_ns` / `duration_ns`.
- `404 Not Found`, `application/json`: `{ "error": "no such sensor log" }` — `id` is not a known live log in the current session. PATCH on a stopped or historical log returns `404`; the daemon does not rewrite manifests outside the live session.

The defining new operation this surface enables: PATCH `retention_ns` from a finite value to `0` to **promote a buffer to a recording** — keep the existing pre-roll on disk, then keep everything from this point forward. The complementary direction (PATCH `retention_ns` from `0` to a finite value) does not retroactively evict; existing segments stay, future evictions follow the new window once `append` runs.

### `DELETE /api/sensor_logs/<id>`

Stop a live log. Closes the underlying `Log<T>` on disk; the log transitions to a stopped state in [`GET /api/sensor_logs`](#get-apisensor_logs) (its `stopped_at_ns` becomes non-null), but **stays listed** for the lifetime of the session — and beyond, since cross-session enumeration finds it on disk thereafter.

**Request.** No body.

**Response.**

- `200 OK`, `application/json`: `{ "stopped": "<sensor_log_id>" }`.
- `404 Not Found`, `application/json`: `{ "error": "no such sensor log" }` — `id` is not a known live log in the current session. DELETE on a stopped or historical log returns `404`; the daemon does not delete already-stopped logs.

DELETE is **not** a destructive operation — it stops writing, it does not remove on-disk segments. Disk-side cleanup is governed by `retention_ns` evictions (only running while the log was live) and any out-of-band cleanup the operator runs against the app-root.

### `POST /api/quit`

Initiate a clean daemon shutdown. The daemon flushes open logs, advertises gracefully (closes mDNS service), and exits.

**Request.** Empty body.

**Response.** `200 OK`, `application/json`: `{ "quitting": true }`.

The HTTP response is sent **before** the daemon begins teardown so the caller doesn't observe a connection error.

---

## Errors

Outside the per-endpoint shapes above, daemons return `application/json` errors:

```json
{ "error": "human-readable message" }
```

Use standard HTTP status codes (`400` malformed input, `404` not found, `409` conflict, `500` server error). The body's `error` field is for operator legibility; consumers should not parse it for branching.

---

## Security model

**Trusted LAN, no authentication.** Daemons bind `0.0.0.0:<port>` with no auth, no TLS, no rate limiting. The assumption is that the LAN segment containing the daemons and their operator UI is itself trusted (private home / lab network, robot's onboard network, etc.).

This is a **deliberate v1 choice** scoped to internal Auki Labs use and trusted-environment deployments. Public-internet exposure of these endpoints is out of scope and not safe under the current spec. When third-party deployments arrive, auth (mTLS, signed bearer tokens, capability certs scoped to a wallet) becomes a v2 concern.

---

## mDNS service discovery

Daemons SHOULD advertise themselves via Multicast DNS so operators don't have to manage URL lists by hand.

| Field           | Value                                                              |
| --------------- | ------------------------------------------------------------------ |
| Service type    | `_auki._tcp.local.`                                                |
| Port            | The port the daemon binds for HTTP (i.e. the same port as `/api/`) |
| TXT `name`      | Operator-friendly identifier (e.g. `k1-walker`, `webcam-front`)    |
| TXT `app`       | Application identifier (e.g. `boosterapp`, `sentinel`)             |

Consumers MAY browse `_auki._tcp.local.` to enumerate daemons. The TXT records `name` and `app` are sufficient to label devices and route control-flow appropriately.

**Manual fallback is mandatory.** Consumers MUST also accept a manual address (`--daemon <url>` CLI flag or equivalent UI input). mDNS isn't always available — VPNs, container networks, restrictive routers — and a no-mDNS path keeps the API usable everywhere.

---

## v1 design choices (deliberate)

The following are intentional v1 simplifications, documented so the next design iteration knows what is "we hadn't gotten to it" vs "we made a call":

- **Poll-based preview.** No MJPEG, no WebSocket, no SSE. Park polls `GET /api/preview/latest.jpg` at whatever rate it wants. Streaming endpoints add implementation complexity and HTTP-server requirements (chunked responses, persistent connections) that v1 isn't ready to commit to. Operator UIs typically poll at 5–10 Hz; that's fine.
- **No authentication.** Trusted-LAN assumption above. Adding auth is the most likely v2 evolution.
- **JSON over HTTP.** No gRPC, no protobuf, no binary framing. Operators inspect with `curl`. Trade some bytes-on-wire for human-debuggability.
- **Single-port HTTP server.** The `image/jpeg` response from `/api/preview/latest.jpg` and the JSON responses from the other endpoints share a port. Daemons aren't expected to spin up separate transports.
- **Spec defines the surface, app defines the lifecycle.** Whether and which sensor logs a daemon auto-creates at session boot (BoosterApp's pre-v0.0.23 30s camera buffer, an optional pointcloud buffer, etc.) is daemon-application policy, not API contract. Auto-created logs appear in [`GET /api/sensor_logs`](#get-apisensor_logs) indistinguishably from client-created ones; the spec says nothing about which (if any) the daemon creates.

---

## Out of scope (for v1)

- **Authentication / TLS / authorization scopes.** Trust model is "trusted LAN."
- **Streaming endpoints.** Preview is poll-only.
- **Rate limiting.** Trusted-environment assumption.
- **Multi-session daemons.** A daemon writes to one *live* session at a time — it can read prior sessions on disk for [`GET /api/sensor_logs`](#get-apisensor_logs), but new logs always go in the live session. Switching the live session = restarting the daemon.
- **Browse-only daemon mode.** A daemon that mounts an app-root *without* opening a live session and serves only the read endpoints. Today the daemon must be running and have a live session for any request to succeed.
- **Cross-daemon coordination.** Each daemon's state is independent; orchestration is the consumer's (Park's) job.
- **Push notifications / webhooks.** No daemon-to-consumer push; consumers poll.
- **The on-disk session shape itself.** That's specified by [`auki-layout`](../crates/auki-layout/README.md) and the per-crate format specs. The Control API operates on top of an existing session; it doesn't define the session.

---

## Versioning

This document is **Control API v1, terminal at SDK release v0.0.23.** The path prefix `/api/` does not encode a version because there will not be an HTTP v2 — control-plane evolution beyond v0.0.23 happens via libp2p protocols (`/auki/control/...`), not new HTTP endpoints. The earlier "if v2 ships, change the path prefix or add a `Server:` header" framing is retired: the trigger for it ("a real v2 use case") has been answered, and the answer was libp2p, not a second HTTP version.

Daemons that conform to this document MUST advertise the data model exactly as specified. Daemons that *also* speak the forthcoming libp2p control protocols MAY do both — the two transports adapt the same in-process API surface and carry the same data shapes.

---

## Implementer's checklist

A daemon is conformant when:

- [ ] Every endpoint above responds with the exact JSON shapes documented.
- [ ] No HTTP endpoint serves `ParticipantInfo` to peers — identity is exchanged only over libp2p `/auki/info/0.0.1`; any app-local identity endpoint is operator-facing/debug, not contract.
- [ ] `POST /api/sensor_logs` validates `sensor_id` / `sensor_hash` against the live session's bindings — `sensor_hash` mismatch returns `409`, unknown `sensor_id` returns `400`.
- [ ] `GET /api/sensor_logs` with no query parameters returns every sensor log the daemon can enumerate, across every session on disk.
- [ ] Each entry in `GET /api/sensor_logs` carries the full per-log shape: `sensor_log_id`, `session_id`, `sensor_id`, `sensor_hash`, `clock_id`, `clock_hash`, `retention_ns`, `duration_ns`, `started_at_ns`, `stopped_at_ns` (`null` only for live logs in the live session).
- [ ] `PATCH /api/sensor_logs/<id>` accepts `retention_ns` and `duration_ns`; rejects mutations to any other field with `400`; returns `404` on stopped or historical logs.
- [ ] `DELETE /api/sensor_logs/<id>` transitions the log to stopped state with non-null `stopped_at_ns`, **keeps it listed** in `GET /api/sensor_logs`, and returns `404` on stopped or historical logs.
- [ ] Registry endpoints serve `<app_root>/registries/{sensors,clocks}/<id>/<hash>.json` verbatim, with `Cache-Control: public, max-age=31536000, immutable`.
- [ ] Daemon registers a session clock at session boot — a fresh monotonic clock per session, on which the session's start is `0` trivially. The `clock_id` and `clock_hash` are the values carried in the daemon's `ParticipantInfo` over libp2p `/auki/info/0.0.1`.
- [ ] No timestamp in any API response is described as "UTC ns" or "monotonic ns" — every timestamp is "ns on the clock identified by `<x>_clock_id`."
- [ ] Errors use `{"error": "..."}` JSON outside the documented per-endpoint response shapes.
- [ ] HTTP server binds `0.0.0.0:<port>`.
- [ ] mDNS advertisement publishes `_auki._tcp.local.` with `name` and `app` TXT records (recommended).
- [ ] Daemon flushes logs and exits cleanly on `POST /api/quit`.
- [ ] `POST /api/quit` responds `200` *before* shutdown begins.
