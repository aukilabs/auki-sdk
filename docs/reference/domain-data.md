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

¹ Imported-session `discover` and data authorization use the original ZITADEL
bearer directly with DDS. DDS is the catalog authority and checks metadata,
data and pose permissions per Domain. Human grants preserve issuer + subject,
organization and atomic scopes, with a maximum five-minute lifetime. The
[provider rollout](../how-to/discover-domains.md#provider-rollout) is required;
no API Domain-store or broad-grant fallback is used for these operations.

Legacy `list` and `accessible_domains` remain compatibility methods for existing
API User grants and the old explicit viewer allowlist bridge. They provide no
permission preview. The corrected `zitadel-identity` viewer profile is rejected
by these methods with a configuration error directing callers to `discover`.
Imported portal-to-Domain association lookup remains unsupported.

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
visibility; it does not report effective read/write permission.
Use `discover(DomainDiscoveryQuery)` to filter by the intersection of data/pose
permissions and receive a real cursor page without tokens or a total count.

`for_portal_page` and `portals_page` return `InventoryPage` (`items`,
`next_cursor`, `paginated`). The first page can acknowledge a bounded complete
response from an older provider with `paginated=false`; continuations must have
matching version/limit metadata. Existing complete-list methods remain.

Data lists filter by `ids`, `name`, and `data_type`. Domain Server data metadata
and pose lists still return complete bounded responses; their pagination is the
remaining Domain Server portion of #385. Oversized responses fail without truncation.

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
replacement-credential persistence. Legacy listing service tokens remain separate from direct DDS human discovery,
data authorization, and P2P requests; each has its own permission contract. Even a cached Domain
grant cannot bypass a pending credential save or a terminal login failure.
Successful exchange, metadata visibility or read permission does not establish
write/delete permission. Discovery permissions are previews, not durable grants.

Rust buffered operations have `_with_cancellation` variants; portal and streaming
methods take a `CancellationToken` directly. Dropping a future stops its request
but cannot await multipart cleanup. `DomainDataClient::close().await` cancels
and drains the client and its clones. Separately created clients and peers
remain usable. The data client has no background refresh loop.

Close all clients and peers before `credential.close().await`. Closing the
session disables its clones and clears cached Domain grants. Task data clients
also lose access when their lease ends; see [task authority](tasks.md#authority-and-peer-lifetime).
