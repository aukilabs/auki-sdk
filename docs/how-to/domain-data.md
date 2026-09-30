# Work with Domain data

Use `AukiDomains` to find Domains and `AukiDomainData` to read and write their
data. Both use your SDK login session and work without starting a peer.

[Sign in](authenticate.md) with a User account, imported ZITADEL session, or
backend App credentials first.
The Rust examples below use that session as `credential` and a Domain UUID
chosen by your app as `selected_domain_id`. For supported credentials, limits,
and errors, see the [Domain data reference](../reference/domain-data.md).

## Select a Domain

If your app needs a Domain picker, request a page of metadata:

~~~rust
use auki_sdk::{AukiDomainData, AukiDomains, DomainListQuery};

let domains = AukiDomains::new(credential.clone());
let page = domains.list(&DomainListQuery {
    limit: 50,
    offset: 0,
    ..Default::default()
}).await?;
~~~

You can also use a known Domain ID without listing. Selecting it creates a
client; the first data operation obtains its access grant:

~~~rust
let data = AukiDomainData::new(credential.clone())?.in_domain(selected_domain_id);
~~~

Domain listing shows what the account can discover. The Domain Server checks
read, write, and delete permission separately.

## Read and write records

Filter records by name, type, or IDs. `list` and `get` return metadata; `read`
returns the stored bytes:

~~~rust
use auki_sdk::DataListQuery;

let records = data.list(&DataListQuery {
    data_type: Some("my-app.report.v1".into()),
    ..Default::default()
}).await?;

if let Some(record) = records.first() {
    let bytes = data.read(record.id).await?;
    // Decode the bytes using your application's format.
}
~~~

Create a record with a name and type. Save the returned ID to replace its
contents or delete it later:

~~~rust
use auki_sdk::DataWrite;

let saved = data.write(
    DataWrite::Named { name: "report", data_type: "my-app.report.v1" },
    b"application-defined bytes",
).await?;

data.write(DataWrite::ById(saved.id), b"replacement bytes").await?;
data.delete(saved.id).await?;
~~~

A buffered named write returns HTTP 409 if the name already exists. Replace
by ID when that is your intent. Buffered transfers default to 8 MiB; use
streaming for larger files.

## Read portals and poses

Look up a portal by UUID or short ID, then read its DDS metadata or its pose
from the selected Domain Server:

~~~rust
use auki_sdk::PortalId;
use tokio_util::sync::CancellationToken;

let cancellation = CancellationToken::new();
let portal = PortalId::parse("ABC12345678")?;
let associated = domains.for_portal(&portal, "own", &cancellation).await?;
let metadata = domains.portal(selected_domain_id, &portal, &cancellation).await?;
let pose = data.pose(&portal, &cancellation).await?;
~~~

To read all records, use `domains.portals(selected_domain_id, &cancellation)`
and `data.poses(&cancellation)`. Portal metadata and pose reads require pose
read permission. These APIs return the service's fields without converting
coordinates or changing the spatial format.

## Read portal pages

`AukiDomains::for_portal_page` and `portals_page` request DDS cursor pages of
1–100 records. Python uses the same names; Web, Swift and Expo use
`forPortalPage` and `portalsPage`. Pass a nonempty `next_cursor` unchanged with
the same filters until it is absent. Bound the number of pages your app follows.

Each page contains `items`, `next_cursor`, and `paginated`. A first response from
an older DDS may contain a bounded complete list with `paginated=false`;
continuation requires versioned pagination acknowledgement. Existing complete
list methods retain their behavior. Selected-Domain portal reads use the existing
Domain grant and pose-read permission. Imported portal-to-Domain association
lookup remains unsupported.

Provider: [DDS #569](https://github.com/aukilabs/domain-service/pull/569).
Deploy pagination support across DDS replicas before relying on continuation.
No authentication migration is required. Domain Server pose/data lists remain
bounded complete responses.

## Stream larger files

Use `read_to` with an async callback that writes each chunk to your destination.
Use `write_stream` with the file's byte length and an async callback that reads
at most the requested number of bytes. The SDK awaits each callback before
requesting the next chunk, so the file need not fit in memory.

Start from the [Rust streaming example](../../core/auki-sdk/examples/domain_data.rs),
[Python file example](../../core/bindings/python/auki-sdk-py/examples/domain_data.py),
or [Web Blob/File helper](../../core/bindings/web/auki-sdk-web/examples/domain-data.ts).
Uploads need a known, nonzero length and a server with multipart uploads enabled.

Multipart completion can replace an existing record by name. Choose a unique
name for a new record, or use its ID to replace one. After a timeout or lost
response, check the record before trying the upload again. See
[streaming behavior](../reference/domain-data.md#streaming) for size limits,
cancellation, and partial-transfer cleanup.

## Use Web or Python

Both bindings expose `domains()` and `data(domain_id)` on the login session.
Reuse a persistent client ID for the application installation.

In Web, data methods return typed metadata and `Uint8Array` bytes:

~~~ts
const session = await AukiUserSession.loginDev(email, password, installationId);
try {
  const data = session.data(selectedDomainId);
  try {
    const records = await data.list({ dataType: "my-app.report.v1" });
    if (records.length) showReport(await data.read(records[0].id));
  } finally {
    await data.close();
  }
} finally {
  await session.close();
}
~~~

In Python, metadata is a dictionary and record contents are `bytes`:

~~~python
from auki_sdk import AukiSession

session = await AukiSession.login_dev(email, password, client_id=installation_id)
try:
    data = session.data(selected_domain_id)
    try:
        records = await data.list(data_type="my-app.report.v1")
        if records:
            report = await data.read(records[0]["id"])
    finally:
        await data.close()
finally:
    await session.close()
~~~

See the [Web](../../core/bindings/web/auki-sdk-web/README.md) and
[Python](../../core/bindings/python/auki-sdk-py/README.md) READMEs for build
instructions and local tests.

## Use the UniFFI facade or Expo

iOS calls `session.domains()` and `session.data(domainId:)` through
[`auki-sdk-uniffi`](../../core/bindings/uniffi/auki-sdk-uniffi/README.md). Android
uses the same crate through generated Kotlin. Expo exports `domains(session)`
and `data(session, domainId)` on Web, iOS, and Android. None of these require a
running peer. They support portal/pose reads, metadata filters, buffered CRUD,
streamed downloads, and multipart uploads.

Start with the [UniFFI facade](../../core/bindings/uniffi/auki-sdk-uniffi/README.md#work-with-domains-and-domain-data)
or the [Expo binding](../../core/bindings/expo/README.md). Web uses the separate
Wasm crate [`auki-sdk-web`](../../core/bindings/web/auki-sdk-web/README.md).
Streaming keeps one bounded chunk in flight across the native bridge and waits
for the destination before continuing. Always cancel and await pending transfers
before closing their client.

## Reuse an imported login

Import your application's ZITADEL PKCE session using the
[authentication guide](authenticate.md#reuse-a-zitadel-login), then use its
Domain picker or create a data client with a known Domain UUID:

~~~ts
const session = AukiUserSession.importZitadelDev(credentials, async replacement => {
  await secureStore.saveAtomically({
    accessToken: replacement.exposeAccessToken(),
    refreshToken: replacement.exposeRefreshToken(),
    clientId: replacement.clientId,
    issuer: replacement.issuer,
    accessTokenExpiresAt: replacement.accessTokenExpiresAt,
  });
});
// Keep session in app state before starting operations, including on failure.
const data = session.data(selectedDomainId);
try {
  const records = await data.list({ dataType: "my-app.report.v1" });
  if (records.length) showReport(await data.read(records[0].id));
} catch (error) {
  // A persistence error is recoverable on this same session after storage recovers.
  // Report the structured error to the application; do not reimport old tokens.
  reportDataError(error);
} finally {
  await data.close();
}
~~~

The SDK exchanges the imported bearer through the ordinary API service-token
route and DDS Domain authentication. It retains rotated credentials until the
host has saved the complete replacement. Data clients and peers share this
refresh owner; stop the application's previous refresh loop before importing.
On logout, close all clients and peers, await `session.close()`, then clear
secure storage.

The same session provides a paged picker through
`session.domains().list({ limit: 50, offset: 0 })`. The SDK uses the ordinary
API-issued User grant for owner and scoped User sessions, applying its
organization and Domain restrictions. Imported viewer grants require the
separate `purpose=p2p` human Domain-allowlist exchange and matching DDS route.
Use default organization selection and no Domain Server filter for imported
sessions. The SDK rejects unsupported token profiles and preserves denials.

The viewer bridge currently enumerates the API's Domain registry before
checking read visibility. A Domain that exists only in DDS can therefore
support known-Domain data access without appearing through that bridge.
Preserve this distinction in your app; a listing failure does not invalidate
the whole session.
Portal-to-Domain association lookup remains unsupported for imported sessions.
Read, write, delete, and pose access remain separate server permission checks;
successful exchange or listing does not authorize an operation.

## Close clients and the shared session

Always await `data.close()` when finished, including after an error. It cancels
and waits for that client's operations, including known multipart upload cleanup.
Close any peers and other clients sharing the session before closing the session
itself. In Rust, preserve the operation's result so cleanup runs before you
return an error:

~~~rust
let result = data.list(&DataListQuery::default()).await;
data.close().await;
credential.close().await;
let records = result?;
~~~

To cancel a Rust operation, use its cancellation token and await the result.
Web accepts an `AbortSignal`; Python uses asyncio cancellation. A cancelled
download can leave a partial file for your app to remove. See
[authentication](authenticate.md) for logout and stored-session cleanup.

## Run a dev round trip

The Rust example signs in to dev, creates a unique record, reads and replaces
it, checks duplicate-name handling, and deletes it. Set `AUKI_EMAIL`,
`AUKI_PASSWORD`, `AUKI_DOMAIN_ID`, and a persistent `AUKI_CLIENT_ID` through
your shell or secret loader; the example does not load `.env`.

Use a dev account and Domain approved for these operations:

~~~sh
cargo run --locked -p auki-sdk --example domain_data
~~~

Add `-- --stream` to test multipart upload and streamed download. Add
`-- --with-peer` to start and stop a direct-only peer using the same session;
this does not book a relay or publish discovery. The flags can be combined.
