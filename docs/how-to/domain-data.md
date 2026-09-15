# Work with Domain data

Use `AukiDomains` to find Domains and `AukiDomainData` to read and write their
data. Both use your SDK login session and work without starting a peer.

[Sign in](authenticate.md) with a User account or backend App credentials first.
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
