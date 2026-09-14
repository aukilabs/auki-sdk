# Work with Domain data

Use `AukiDomains` and `AukiDomainData` without starting networking. They share an
`AukiCredential` (the existing `AuthSession`) with `AukiPeerBootstrap`.
User password login works in Rust, Web and Python. App key/secret login is
available in native Rust and Python and belongs on trusted backends.
The existing Web `AukiUserSession` and Python `AukiSession` expose `domains()`
and `data(domain_id)` without starting a peer.

```rust,no_run
use auki_sdk::{
    AukiDomainData, AukiDomains, AukiPeerBootstrap, AukiPeerConfig, AuthClient,
    AuthEnvironment, Credentials, DataListQuery, DataWrite, DomainListQuery,
};

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let env = AuthEnvironment::dev()
    .with_client_id(std::env::var("AUKI_CLIENT_ID")?)?;
let credential = AuthClient::new(env)?
    .authenticate(Credentials::user_password(
        std::env::var("AUKI_EMAIL")?, std::env::var("AUKI_PASSWORD")?,
    )).await?;
let result: Result<(), Box<dyn std::error::Error>> = async {
    let domains = AukiDomains::new(credential.clone());
    let page = domains.list(&DomainListQuery { limit: 50, offset: 0, ..Default::default() }).await?;
    // The caller selects a Domain; a known ID also works without listing.
    let domain_id = std::env::var("AUKI_DOMAIN_ID")?.parse()?;
    let data = AukiDomainData::new(credential.clone())?.in_domain(domain_id);
    let records = data.list(&DataListQuery {
        data_type: Some("my-app.report.v1".into()), ..Default::default()
    }).await?;
    if let Some(record) = records.first() {
        let metadata = data.get(record.id).await?;
        let bytes = data.read(metadata.id).await?;
    }
    let saved = data.write(DataWrite::Named {
        name: "report", data_type: "my-app.report.v1",
    }, b"application-defined bytes").await?;
    data.write(DataWrite::ById(saved.id), b"replacement bytes").await?;
    data.delete(saved.id).await?;
    data.close().await;
    // Optional networking uses this same credential; construction starts no peer.
    let bootstrap = AukiPeerBootstrap::from_session(credential.clone(), AukiPeerConfig::dev());
    Ok(())
}.await;
credential.close().await;
result?;
# Ok(())
# }
```

Persist one `AUKI_CLIENT_ID` per application installation and reuse it across
logins. DDS uses this value for access accounting. Without an explicit value,
the environment generates an ID shared by its clones for that environment's
lifetime. Metadata listing sends `issue_token=false` and SDK identification;
it does not obtain potentially billed tokens for each picker entry.

For another environment, use `AuthEnvironment::new(api, dds)` with aligned
endpoints. The selected Domain's server URL comes from authenticated DDS.
The client caches renewable data grants by credential and Domain, checks their
issuer/Domain/audience/expiry before sending, and obtains one replacement after a 401.
P2P uses its own signed peer-bound credentials. The Domain Server checks data
signatures and permissions on every request; successful discovery or P2P
admission does not establish write/delete access.

## Data and pagination semantics

- `DomainListQuery` supports `own` (default), an organization UUID, and `all`
  only with an owned Domain Server. It uses real DDS `limit`/`offset` pages
  (1–100). Totals may change between pages; they are not snapshot guarantees.
- Listing a Domain establishes visibility, not effective read/write permission.
  Permission filters require backend issue #383 and are not exposed here.
- Data lists support `ids`, `name` and `data_type`, returning metadata only.
  They have no server pagination; oversized responses fail explicitly.
- Named writes use the simple POST upload endpoint, which returns 409 for a
  duplicate name. ID replacement preserves the name/type and fails for a missing
  ID. No lookup-and-overwrite fallback runs on conflict. The server's separate
  multipart-session completion path can replace by name; it is not used here.
  There is no version history, automatic merging or replay after a lost response.
  Names/types are 1–128 bytes and follow the server's punctuation restrictions;
  `/` is disallowed, so use a type such as `my-app.report.v1`.
- Buffered transfers default to 8 MiB of data and 1 MiB of metadata, configurable
  through `DataLimits` up to 64 MiB/4 MiB. Uploads also check the selected server's
  `/api/v1/info` request/file limits, including multipart envelope overhead.
  For larger files, use the streaming helpers below. Buffered writes do not
  silently switch upload contracts based on size.
- `DataError::status()` preserves HTTP distinctions such as 401, 402, 403, 404,
  413, 429 and 5xx. Bodies, tokens and application data are excluded from errors.
  Retries are limited to a single confirmed 401; a timeout, cancellation or
  connection loss after a write may leave its outcome unknown. Reconcile it
  before resubmitting.

## Cancellation and shutdown

Every data operation has a `_with_cancellation` variant taking a
`tokio_util::sync::CancellationToken`. Dropping the future stops its request.
`DomainDataClient::close().await` cancels and drains that client's operations,
including its clones. It leaves separately created clients and peers usable.
There are no background data tasks or refresh loops.

The application closes all users of the credential before calling
`credential.close().await`. This fences all session clones and clears cached
Domain grants. See [peer cleanup](lifecycle.md) and [authentication](authenticate.md)
for logout and imported-session persistence. Imported ZITADEL data operations
return an explicit unsupported-configuration error in this milestone, without
refreshing or changing stored credentials; existing P2P import behavior is unchanged.

## Try the dev round trip

The [example](../../core/auki-sdk/examples/domain_data.rs) requires `AUKI_EMAIL`,
`AUKI_PASSWORD`, a selected `AUKI_DOMAIN_ID`, and a persistent `AUKI_CLIENT_ID`.
It signs in to dev, lists Domains, creates a uniquely named small data record,
reads it, replaces it by ID, checks duplicate-name conflicts, and deletes it. Cleanup reconciles by the
unique name even when a write response is lost. It leaves existing records alone.

Run only with an approved dev account/Domain that permits these operations:

```sh
cargo run --locked -p auki-sdk --example domain_data
# Also start and stop a direct-only peer sharing the credential:
cargo run --locked -p auki-sdk --example domain_data -- --with-peer
```

The optional peer uses no relay booking or discovery publication. It verifies
shared authentication/lifecycle, not a two-peer exchange. No environment file is
loaded automatically; supply credentials through your shell or secret loader.

## Read portals and poses

```rust,no_run
# async fn example(domains: &auki_sdk::AukiDomains, data: &auki_sdk::DomainDataClient, domain_id: uuid::Uuid) -> Result<(), Box<dyn std::error::Error>> {
let cancel = tokio_util::sync::CancellationToken::new();
let portal = auki_sdk::PortalId::parse("ABC12345678")?; // UUID or short ID
let associated = domains.for_portal(&portal, "own", &cancel).await?;
let portals = domains.portals(domain_id, &cancel).await?;
let metadata = domains.portal(domain_id, &portal, &cancel).await?;
let poses = data.poses(&cancel).await?;
let pose = data.pose(&portal, &cancel).await?;
# Ok(())
# }
```

Portal association lookup uses the User/App service session and sends
`issue_token=false`. Unlike ordinary Domain listing, portal lookup allows `all`
without specifying a Domain Server. Neither endpoint establishes write permission.

Portal records come from DDS; poses come from the selected Domain Server. Both
read routes require **pose read** permission. The client uses the selected Domain
grant and checks its DDS audience before sending it to the configured DDS host.
The underlying service routes are still named `lighthouses`. No spatial format,
coordinate conversion, pose creation or portal assignment is introduced. These
lists are bounded complete responses, with no invented pagination.

## Stream larger files

`read_to(id, options, cancellation, sink)` delivers byte chunks to an awaited
callback. `write_stream(target, size, options, cancellation, source)` asks a
callback for at most a specified number of bytes at a time. Callbacks are serial,
so the caller controls backpressure; the SDK never collects a whole large file.
Use an asynchronous file reader/writer or adapt another source/sink in your app.
Rust callbacks return `Result<_, DataError>`; map local I/O errors to `Callback`
and retain private details in your own application error handling.

- Streaming defaults to a maximum file size of 8 GiB and a maximum chunk/part
  allocation of 16 MiB (configurable up to 64 MiB). These are separate from the
  buffered API's 8 MiB default. Memory is bounded by the part/chunk size, transport
  buffers and at most 10,000 part receipts, not the total file size.
- Uploads require a nonzero known byte length, as provided by file metadata or
  `Blob.size`. The source returns empty bytes at EOF. Early EOF, extra bytes and
  callbacks exceeding their requested maximum fail explicitly.
- Before initiating, the client reads the selected server's public `/api/v1/info`
  and checks its file limit, request limit, multipart support and part size.
  The initiation response supplies the actual part size and session expiry.
  Servers requiring parts larger than the configured memory bound fail explicitly.
- Multipart named completion **can replace by name**. This is the service's
  multipart contract; buffered named POST still returns 409 on duplicates.
  Choose a unique name for new data or an explicit ID for replacement.
- Tokens renew between requests and once after a confirmed 401. Transfers remain
  pinned to their initial server. Parts/completion are not replayed after a
  timeout or lost connection; downloads never restart after delivering bytes.
- The client timeout applies to each request, read and callback, allowing long
  transfers that continue making progress. Server multipart expiry still applies.
- Cancel and await the operation. Known multipart sessions receive an awaited,
  bounded abort attempt even on client close or source failure. A cleanup failure
  is reported alongside the original error. An unknown upload ID after a lost
  initiation response, an abandoned Rust future or a terminated process must
  rely on server session expiry. There is no resume-across-restarts guarantee.
- A cancelled download may have written a partial destination. The application
  owns its removal or reconciliation. After ambiguous upload completion, reconcile
  by the unique name or known ID before resubmitting.

The Rust dev example accepts `-- --stream` (also combinable with `--with-peer`)
to replace its temporary record using three multipart parts, stream it back,
verify every byte and read existing portal/pose records.

## Web and Python

Web methods return typed metadata objects and `Uint8Array` bytes. Rejections are
`Error` objects with `kind` and an optional HTTP `status`. Pass an `AbortSignal`
to cancel; source/sink callbacks also receive a signal for their own pending work.
Close data clients before closing the shared login.

```ts
const session = await AukiUserSession.loginDev(email, password, installationId);
const domains = session.domains();
const page = await domains.list({ limit: 50 });
const data = session.data(selectedDomainId);
try {
  const records = await data.list({ dataType: "my-app.report.v1" });
  if (records.length) showReport(await data.read(records[0].id));
} finally {
  await data.close();
  await session.close(); // after stopping any peers sharing it
}
```

The [Web Blob round trip](../../core/bindings/web/auki-sdk-web/examples/domain-data.ts)
uses `writeStream` and `readTo`, including cancellation and temporary-record cleanup.
It accepts an existing session, selected Domain, `Blob`/`File`, and destination
callback. Compile it with the binding's `npm run check`; adapt its package import
when embedding it in your own bundler. Browser CORS policy still applies.

Python metadata is returned as dictionaries; data is `bytes`. `DomainDataError`
retains `kind` and `status`. Streaming callbacks are `async` functions. Cancelling
an asyncio operation signals its native task and cancels its pending Python
callback; `await data.close()` drains the transfer's multipart cleanup.

```python
session = await AukiSession.login_dev(email, password, client_id=installation_id)
data = session.data(selected_domain_id)
try:
    page = await session.domains().list(limit=50)
    records = await data.list(data_type="my-app.report.v1")
    if records:
        report = await data.read(records[0]["id"])
finally:
    await data.close()
    await session.close()
```

The runnable [Python file round trip](../../core/bindings/python/auki-sdk-py/examples/domain_data.py)
requires Python 3.9+ for `asyncio.to_thread`. It generates a 17 MiB file by default,
compares upload/download SHA-256 and deletes its unique dev record. Pass an input
path to use your own file. It reads credentials from the same `AUKI_*` environment
variables as the Rust example; it does not load `.env` automatically.

## Existing Posemesh consumers

This is an additive SDK API. Existing `posemesh-domain-http`, `@auki/domain-client`
and Python clients remain available, including their administration and task
helpers. Rust callers can migrate data/discovery operations to the types above
and replace independent login calls with a cloned SDK session. Web/Python callers
can migrate through their existing SDK session's `domains()` and `data()` methods.
Optional client IDs on login preserve existing callers and let installations
retain their accounting identity across logins. The server's names, IDs, types,
bytes and portal/pose fields retain their meaning.

Swift/Expo data bindings, machine/task credential adapters (#375), and imported
ZITADEL data integration remain follow-ups. Permission-aware discovery, identity
reconciliation and additional pagination depend on #383–#385. No backend wire
format, endpoint, permission or deployment change is required for these additions.
