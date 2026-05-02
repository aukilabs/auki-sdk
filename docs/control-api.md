# Cross-app HTTP Control API — v1

The Auki SDK's operator-control surface. Daemons that produce SDK sessions (BoosterApp, Sentinel, future) implement this API so any UI — primarily Park — can drive any of them through a single contract.

This is **not** part of the data protocol. Data products flow through the SDK's on-disk format (registries, logs, segments) and the (forthcoming) peer-discovery layer. The Control API is a runtime, operator-facing concern: list recordings, start/stop them, peek at the latest captured frame, change buffer retention, request a clean shutdown.

## Conformance

A daemon is **Auki Control API v1 conformant** when it:

1. Implements every endpoint below with the exact request/response shapes specified.
2. Returns a JSON `{"error": "<message>"}` body for any non-success status code outside the response shapes specified per endpoint.
3. (Optional but recommended) Advertises itself via mDNS per the convention below.

Consumers (Park, etc.) MAY auto-discover daemons via mDNS and MUST also support a manual fallback (`--daemon <url>` or equivalent).

---

## Endpoints

All endpoints live under `/api/`. Daemons bind `0.0.0.0:<port>` — no authentication, trusted-LAN assumption (see [Security model](#security-model) below).

### `GET /api/info`

Daemon identity. Operator-facing label and the implementing application's name, suitable for use as a UI label when a consumer reaches the daemon by URL rather than mDNS.

**Request.** No body.

**Response.** `200 OK`, `application/json`:

```json
{
  "name": "k1-walker",
  "app": "boosterapp"
}
```

| Field  | Type   | Notes                                                         |
| ------ | ------ | ------------------------------------------------------------- |
| `name` | string | Operator-friendly identifier. MUST match the `name` TXT record when the daemon also advertises via mDNS. |
| `app`  | string | Application identifier. MUST match the `app` TXT record when the daemon also advertises via mDNS. |

Daemons that don't run mDNS still implement this endpoint; the values are operator-configurable strings (`--device-name`, defaults to hostname / binary name when unset).

### `GET /api/state`

Snapshot of the daemon's current session and all recordings, active and stopped.

**Request.** No body.

**Response.** `200 OK`, `application/json`:

```json
{
  "session_uuid": "abc-123-...",
  "recordings": [
    {
      "recording_id": "rec-0",
      "retention_ns": 30000000000,
      "started_at_ns": 1745000000000000000,
      "stopped_at_ns": null,
      "duration_ns": 30000000000,
      "frame_count": 612,
      "sensor_id": "K1-AABBCCDDEEFF/head_left_cam",
      "sensor_hash": "abc123...",
      "clock_id": "K1-AABBCCDDEEFF/utc",
      "clock_hash": "def456..."
    },
    {
      "recording_id": "rec-1",
      "retention_ns": 0,
      "started_at_ns": 1745000045000000000,
      "stopped_at_ns": 1745000165000000000,
      "duration_ns": 120000000000,
      "frame_count": 3640,
      "sensor_id": "K1-AABBCCDDEEFF/head_left_cam",
      "sensor_hash": "abc123...",
      "clock_id": "K1-AABBCCDDEEFF/utc",
      "clock_hash": "def456..."
    }
  ]
}
```

| Field                          | Type             | Notes                                                              |
| ------------------------------ | ---------------- | ------------------------------------------------------------------ |
| `session_uuid`                 | string           | The session this daemon is currently writing to.                   |
| `recordings`                   | array            | All recordings of this session — active *and* stopped — ordered by `started_at_ns` ascending. |
| `recordings[].recording_id`    | string           | Daemon-assigned identifier; opaque to consumers.                   |
| `recordings[].retention_ns`    | integer          | Retention window for this recording. `0` = unbounded.              |
| `recordings[].started_at_ns`   | integer          | Wall-clock UTC ns when the recording was opened.                   |
| `recordings[].stopped_at_ns`   | integer \| null  | Same clock as `started_at_ns`, set the moment `DELETE /api/recordings/<id>` was processed. `null` while the recording is active. Recording state is determined by this field: `null` = active, non-null = stopped. |
| `recordings[].duration_ns`     | integer          | Footage currently held in this recording. Computed by the daemon: `min(now - started_at_ns, retention_ns)` for an active ring buffer, `now - started_at_ns` for an active unbounded recording, `stopped_at_ns - started_at_ns` once stopped. |
| `recordings[].frame_count`     | integer          | Frames written to this recording so far.                           |
| `recordings[].sensor_id`       | string           | The sensor this recording streams from. Resolves via [`GET /api/registries/sensors/<sensor_id>/<sensor_hash>`](#get-apiregistriessensorssensor_idsensor_hash). |
| `recordings[].sensor_hash`     | string           | The content-addressed hash pinning the exact sensor entry the recording was opened against. The hash IS the version — don't substitute.                                            |
| `recordings[].clock_id`        | string           | The clock used for the recording's per-frame timestamps. Resolves via [`GET /api/registries/clocks/<clock_id>/<clock_hash>`](#get-apiregistriesclocksclock_idclock_hash). |
| `recordings[].clock_hash`      | string           | The content-addressed hash pinning the exact clock entry. Same hash-is-version rule.                                                  |

**Important shape decisions.**

- The auto-started ring buffer is `recordings[0]` with `retention_ns: 30000000000` (30 s default). It is **not** a separate `buffer` field. Daemons distinguish the buffer from intent recordings only by its `retention_ns` value. There can be exactly one auto-started buffer (started at session boot), or zero (some operator stopped it); intent recordings can be any number.
- Stopped recordings stay in the `recordings` array for the lifetime of the session — they transition from `stopped_at_ns: null` to a non-null value, but they don't disappear. The list is "all recordings of this session," not "currently active recordings." Consumers wanting only active recordings filter on `stopped_at_ns == null`. Daemon restart resets the session (new `session_uuid`) and drops the in-memory list — stopped recordings remain on disk under `<session>/sensorlogs/<recording-id>/` regardless.

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

A future Frame Registry endpoint will follow the same shape (`/api/registries/frames/<frame_id>/<frame_hash>`); not yet specified — the on-disk Frame Registry is still pending in the SDK.

### `GET /api/preview/latest.jpg`

Most recent frame captured by the daemon, encoded as JPEG. Poll-based — see [v1 design choices](#v1-design-choices-deliberate) for why no streaming.

**Request.** No body.

**Response.**

- `200 OK`, `image/jpeg` — the latest frame.
- `503 Service Unavailable` — no frame captured yet (daemon just started, no source data).

### `POST /api/recordings`

Open a new intent recording. Always unbounded (`retention_ns: 0`); buffer-style retentions are managed via [`PATCH /api/buffer`](#patch-apibuffer) on the auto-started buffer, not by creating new recordings here.

**Request.** Empty body.

**Response.** `201 Created`, `application/json`:

```json
{ "recording_id": "rec-1" }
```

### `DELETE /api/recordings/<id>`

Stop a specific recording. Closes the log on disk; the recording transitions to a stopped state in `GET /api/state` (its `stopped_at_ns` becomes non-null and its `duration_ns` freezes at the final value), but **stays in the `recordings` array** for the lifetime of the session.

**Request.** No body.

**Response.**

- `200 OK`, `application/json`: `{ "stopped": "rec-1" }`
- `404 Not Found`, `application/json`: `{ "error": "no such recording" }` — `id` is unknown, or refers to a recording that is already stopped (DELETE is idempotent only in the sense that re-stopping a stopped recording is a no-op error; the on-disk state doesn't change).

Stopping the auto-started buffer (`recordings[0]`) is permitted — the buffer enters the stopped state like any other recording.

### `PATCH /api/buffer`

Change the auto-started buffer's retention window at runtime.

**Request.** `application/json`:

```json
{ "retention_ns": 60000000000 }
```

**Response.**

- `200 OK`, `application/json`: `{ "retention_ns": 60000000000 }` (echoes the new value).
- `400 Bad Request`, `application/json`: `{ "error": "<message>" }` for malformed body, negative `retention_ns`, etc.

Acts on the auto-started buffer only (`recordings[0]`). If no buffer exists (it was stopped), this returns `400`.

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
- **Buffer is `recordings[0]` (and only [0]).** A daemon has at most one auto-started buffer per session. Multiple concurrent buffers (different retentions for different recordings) is conceptually possible but adds API surface — out of scope.

---

## Out of scope (for v1)

- **Authentication / TLS / authorization scopes.** Trust model is "trusted LAN."
- **Streaming endpoints.** Preview is poll-only.
- **Rate limiting.** Trusted-environment assumption.
- **Multi-session daemons.** A daemon writes to one session at a time. Switching sessions = restarting the daemon.
- **Multiple buffers per session.** One auto-started buffer; intent recordings are individually unbounded.
- **Cross-daemon coordination.** Each daemon's state is independent; orchestration is the consumer's (Park's) job.
- **Push notifications / webhooks.** No daemon-to-consumer push; consumers poll.
- **The on-disk session shape itself.** That's specified by [`auki-session`](../crates/auki-session/README.md) and the per-crate format specs. The Control API operates on top of an existing session; it doesn't define the session.

---

## Versioning

This document is **Control API v1**. The path prefix `/api/` does not encode a version — daemons advertise a single API surface at a single version. When v2 ships:

- Either the path prefix changes (`/api/v2/...`), OR
- A `Server: auki-control/2` HTTP header is required for negotiation, OR
- A daemon that supports multiple versions exposes them at distinct ports.

Decision deferred until a real v2 use case appears. Until then, `/api/` ≡ v1 ≡ this document.

---

## Implementer's checklist

A daemon is conformant when:

- [ ] Every endpoint above responds with the exact JSON shapes documented.
- [ ] `recordings[0]` is the auto-started buffer with `retention_ns: 30000000000` by default.
- [ ] Each recording in `/api/state` carries `sensor_id` + `sensor_hash` + `clock_id` + `clock_hash` matching the on-disk manifest.
- [ ] Each recording in `/api/state` carries `stopped_at_ns` (`null` while active) and `duration_ns` (computed per the table above) on the same clock as `started_at_ns`.
- [ ] `DELETE /api/recordings/<id>` transitions the recording to stopped state but **keeps it in the `recordings` array** with non-null `stopped_at_ns`.
- [ ] Registry endpoints serve `<app_root>/registries/{sensors,clocks}/<id>/<hash>.json` verbatim, with `Cache-Control: public, max-age=31536000, immutable`.
- [ ] `/api/info`'s `name` / `app` match the mDNS TXT records when both are configured.
- [ ] Errors use `{"error": "..."}` JSON outside the documented per-endpoint response shapes.
- [ ] HTTP server binds `0.0.0.0:<port>`.
- [ ] mDNS advertisement publishes `_auki._tcp.local.` with `name` and `app` TXT records (recommended).
- [ ] Daemon flushes logs and exits cleanly on `POST /api/quit`.
- [ ] `POST /api/quit` responds `200` *before* shutdown begins.
