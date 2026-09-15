# Domain data reference

For examples, see [Work with Domain data](../how-to/domain-data.md).
The SDK reexports the types from
[`auki-domain-client`](../../core/auki-domain-client/README.md).

## Platforms and credentials

| Credential | Rust | Python | Web | Swift/iOS | Expo Web/iOS |
| --- | --- | --- | --- | --- | --- |
| User email/password | Listing and data | Listing and data | Listing and data | Listing and data | Listing and data |
| App key and secret | Native listing and data | Listing and data | Not exposed | Not exposed | Not exposed |
| Imported ZITADEL session | Listing and data¹ | Listing and data¹ | Listing and data¹ | Listing and data¹ | Listing and data¹ |
| Compute or robot task lease | Native, through `task.data()` | `task.data()` | Not exposed | Not exposed | Not exposed |
| Robot, while idle | Native, assigned Domain reads | Assigned Domain reads | Not exposed | Not exposed | Not exposed |

App secrets belong on trusted backends. Machine credentials get their Domain
from a lease or robot assignment; they do not provide a user Domain picker.
Android is not implemented by the Expo binding.

Imported-session data uses the existing ordinary API service exchange, then
DDS selected-Domain authentication. The deployment must accept imported
ZITADEL bearers on that API route.

¹ Imported owner and scoped User grants use the ordinary service exchange and
the same DDS listing route as User login. The API-issued `user-access` grant
limits the request to its organization and any explicit Domain restrictions;
a null or empty restriction list allows owned Domains.

The older `accessible_domains` picker follows DDS User access-control rules
and can also include public Domains or Domains explicitly shared with the
token's organization. Any explicit token Domain restrictions still apply;
its total can differ from the owned-only `domains().list()` query.

Imported viewer grants are App-shaped and cannot safely use legacy listing.
They require `POST /service/domains-access-token?purpose=p2p` to issue a
`user-p2p-access` token with an explicit, nonempty human Domain allowlist,
followed by DDS `/api/v1/accessible-domains`. Older providers that ignore this
purpose fail closed. The SDK checks each page against its grant and preserves
permission errors. The viewer bridge's API Domain registry can differ from
DDS, so known-Domain data can work while that listing is denied. Portal-to-Domain
association queries remain unsupported for imported sessions. See
[provider compatibility](../../test-support/domain-data-validation.md#provider-compatibility).

`AukiCredential` is an alias for `AuthSession`. A User, App, or imported session can be
shared by data clients and `AukiPeerBootstrap`; data access does not require
a peer or DMS configuration. Native task and robot credentials provide data
access through the [task runtime](tasks.md).

Persist one client ID per application installation and reuse it across logins.
DDS uses it for access accounting. `AuthEnvironment::with_client_id` sets it
in Rust; Web, Swift, and Expo login accept `clientId`, and Python login accepts
`client_id`.
Without an explicit ID, an auth environment generates one shared by its clones
for that environment's lifetime.

## APIs and responses

| Rust API | Result |
| --- | --- |
| `AukiDomains::list` | Page of Domain metadata |
| `AukiDomains::for_portal` | Domains associated with a portal UUID or short ID |
| `AukiDomains::portals`, `portal` | DDS portal metadata |
| `AukiDomainData::in_domain` | A `DomainDataClient`; no request is made |
| `DomainDataClient::list`, `get` | Data metadata |
| `read`, `read_to` | Buffered bytes or streamed chunks |
| `write`, `write_stream` | Metadata for the stored record |
| `delete` | Removal of the selected record |
| `poses`, `pose` | Pose records from the selected Domain Server |

`DataMetadata` contains `id`, `domain_id`, `name`, `data_type`, `size`,
`created_at`, and `updated_at`. The client preserves the service's IDs, types,
bytes, and portal/pose fields. Web returns typed objects and `Uint8Array`;
Python returns dictionaries and `bytes`. Swift exposes records and `Data`;
Expo returns typed objects and `Uint8Array`.

### Listing and permissions

For User/App sessions, `DomainListQuery` accepts `own` (the default), an
organization UUID, or `all` with an owned Domain Server. Imported sessions
accept the default `own` query without a Domain Server filter. DDS uses `limit`
and `offset`; limits are 1–100.
Totals can change between pages. Portal association lookup also accepts `all`
without a Domain Server filter.

User/App Domain and portal association listings send `issue_token=false` and SDK
identification. Imported User grants use the same Domain route; imported viewer
grants use the allowlist metadata route. Neither
obtains potentially billed Domain tokens for every entry. Listing establishes
visibility; it does not report effective read/write
permission. Permission filters are not available.

Data lists filter by `ids`, `name`, and `data_type`. They return metadata without
server pagination. Portal and pose lists also return complete, bounded
responses; oversized responses fail rather than being truncated.

Portal metadata comes from DDS and poses come from the Domain Server. Both
read routes require pose read permission. The underlying routes are named
`lighthouses`. The client does not create poses, assign portals, or convert
coordinates.

### Writes and conflicts

| Target | Buffered `write` | Multipart `write_stream` |
| --- | --- | --- |
| `DataWrite::Named` | Creates a record; duplicate name returns HTTP 409 | Completion can replace by name |
| `DataWrite::ById` | Replaces an existing record's contents, preserving name and type | Replaces by ID |

Buffered writes never switch to multipart sessions based on size or try a
lookup-and-overwrite after a conflict. A missing replacement ID fails. Names
and types must be 1–128 bytes and satisfy the server's character restrictions;
`/` is disallowed. For example, use `my-app.report.v1` as a type.

There is no version history, automatic merging, or replay after a lost response.

## Limits

| Setting | Default | Maximum |
| --- | --- | --- |
| `DataLimits::max_data_bytes` | 8 MiB | 64 MiB |
| `DataLimits::max_metadata_bytes` | 1 MiB | 4 MiB |
| `DataLimits::request_timeout` | 30 seconds | 300 seconds |
| `TransferOptions::max_bytes` | 8 GiB | `i64::MAX` bytes |
| `TransferOptions::max_chunk_bytes` | 16 MiB | 64 MiB |
| Multipart part receipts | — | 10,000 |

Use `AukiDomainData::with_limits` for buffered limits and timeouts, and
`TransferOptions` for streaming bounds. The selected server's file and request
limits also apply, including multipart envelope overhead.

## Streaming

`read_to(id, options, cancellation, sink)` awaits each destination callback.
`write_stream(target, size, options, cancellation, source)` requests at most
the supplied maximum from its source callback. Callbacks are serial. Memory
use depends on chunk/part size, transport buffers, and part receipts, rather
than total file size.

Uploads require a known, nonzero length. Return empty bytes at EOF; early EOF,
extra bytes, and callbacks exceeding the requested maximum fail. Before upload,
the client reads `/api/v1/info` to check file limits, request limits, multipart
support, and part size. Initiation supplies the actual part size and session
expiry. A part size above the configured memory bound fails.

Transfers stay on their initial server. Credentials renew between requests
and once after a confirmed HTTP 401. The timeout applies to each request, read,
and callback, so a long transfer can continue while making progress. Server
session expiry still applies.

Parts and completion requests are not replayed after a timeout or lost
connection. Downloads never restart after delivering bytes. Cancelled downloads
can leave a partial destination. After an uncertain upload outcome, reconcile
by the unique name or known ID before resubmitting.

Cancel and await the operation to abort a known multipart session. Cleanup is
bounded and awaited, including on client close or source failure; a cleanup
error is reported with the original error. Lost initiation responses, abandoned
Rust futures, and terminated processes rely on server session expiry. Uploads
cannot resume across process restarts.

Rust callbacks return `Result<_, DataError>`; map local I/O failures to
`DataError::Callback` and retain private error details in your application.
Web `readTo`/`writeStream` callbacks receive an `AbortSignal` for pending work.
Python callbacks are async; cancelling an asyncio operation signals the native
transfer and cancels its pending Python callback. Swift and Expo expose bounded
stream adapters over the same Rust transfer implementation; see their binding
READMEs for cancellation and cleanup.

## Errors, authority, and shutdown

Rust `DataError::status()`, Web `Error.status`, and Python
`DomainDataError.status` retain HTTP status when present. Web, Python, and Expo
also expose `kind` and an optional authentication `code`; Swift's Domain data
error carries the equivalent status and authentication kind. A `persistence`
failure requires retrying with the retained session to save its replacement
credentials. Statuses such as 401, 402, 403, 404, 413, 429, and 5xx remain
distinct. Errors exclude response bodies, credentials, and application data.
Only a confirmed 401 permits one authenticated retry. A timeout, cancellation,
or connection loss after a write can leave its outcome unknown.

For User/App/imported sessions, the selected server URL comes from authenticated DDS.
The client caches renewable grants by credential and Domain and checks issuer,
Domain, audience, and expiry before use. DDS portal reads also check the DDS
audience before sending a grant to that host. Domain Servers verify signatures
and permissions. P2P admission and discovery do not grant data write access.
Keep service endpoints aligned to one environment; browser CORS rules apply.

Imported listing, data clients, and peers use one refresh owner and await complete
replacement-credential persistence. Listing and data service tokens remain
separate from the imported bearer used by direct DDS P2P authentication. Even a cached Domain
grant cannot bypass a pending credential save or a terminal login failure.
The existing API role projection is coarse: successful exchange, visibility,
or read access does not establish write/delete permission.

Rust buffered operations have `_with_cancellation` variants; portal and streaming
methods take a `CancellationToken` directly. Dropping a future stops its request
but cannot await multipart cleanup. `DomainDataClient::close().await` cancels
and drains the client and its clones. Separately created clients and peers
remain usable. The data client has no background refresh loop.

Close all clients and peers before `credential.close().await`. Closing the
session disables its clones and clears cached Domain grants. Task data clients
also lose access when their lease ends; see [task authority](tasks.md#authority-and-peer-lifetime).
