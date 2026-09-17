# Auki networking for Swift

Use `AukiSession` and `AukiPeer` to connect your iOS app to the Auki network.
The local Swift package requires Swift 6, iOS 17+, Xcode, and Rust 1.89+.

Build from the SDK repository root:

~~~sh
rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios
bash core/bindings/swift/auki-sdk-swift/build-xcframework.sh
~~~

Add this directory's local Swift package to your app. The build includes
experimental protocol bindings. Register the handlers you want to use.

The `urdf-fk` feature (included in `standard-protocols`) uses the in-repository
[`auki-urdf-fk`](../../../../labs/auki-urdf-fk/README.md) crate to parse URDF XML
and resolve link transforms. Supply robot descriptions and meshes from your app;
no separate `auki-libs` checkout is needed.

Compile the SDK and your Rust adapter into one framework, as in
[Swift Echo](../../../examples/portable-echo/swift/README.md).
See [custom protocols](../../../../docs/how-to/protocols.md).

Run the offline Swift checks with:

~~~sh
core/bindings/swift/auki-sdk-swift/run-domain-data-bindings-test.sh
core/bindings/swift/auki-sdk-swift/run-jobs-bindings-test.sh
~~~

The Domain data check uses a loopback HTTP fixture. The jobs check exercises
the typed codec against a fake generated UniFFI object.

## Work with Domains and Domain data

Persist one opaque client ID for the app installation and reuse it across
logins. Domain discovery and data access share the session's User credentials;
they do not start a peer or contact DMS:

~~~swift
let session = try await AukiSession.loginDev(
    email: email,
    password: password,
    clientId: installationID
)

let page = try await session.domains().list(
    query: AukiDomainListQuery(limit: 50),
    cancellation: nil
)
guard let selectedDomain = page.domains.first else {
    await session.close()
    return
}
let data = try session.data(domainId: selectedDomain.id)

do {
    let records = try await data.list(
        query: AukiDataListQuery(dataType: "my-app.report.v1"),
        cancellation: nil
    )
    if let record = records.first {
        let bytes = try await data.read(dataId: record.id, cancellation: nil)
        // Decode the application-defined bytes.
    }
    try await data.close()
    await session.close()
} catch {
    try? await data.close()
    await session.close()
    throw error
}
~~~

`list` returns one real server page. Advance `offset` by the returned number of
Domains until it reaches the advisory `total`; totals can change between pages.
Portal lookup accepts a UUID or eleven-character short ID. `portals`/`portal`
return DDS metadata, while `poses`/`pose` return the Domain Server's unchanged
pose fields.

An imported ZITADEL session uses the same awaited credential store for Domain
listing, data access, and peer authentication. Its default `domains().list`
query uses the API's ordinary User access profile for owners and other User
roles, then validates the DDS page against the token's organization and any
Domain allowlist. Viewer profiles use the API's narrower P2P listing grant.
Organization and Domain Server filters and portal association queries remain
unsupported for imported sessions. A 403 listing denial is recoverable: the
same session may still access a known Domain ID if the provider authorizes it.
Keep the session on a
`.DomainData(... authKind: .persistence ...)` error and retry after secure
storage is available. `loginDataWithEnvironment` and
`importZitadelDataWithEnvironment` take only API and DDS URLs for apps that do
not need peer networking. App access-key secrets are not exposed by this mobile
binding.

## Stream files with bounded memory

Buffered reads and writes default to 8 MiB. The transfer handles keep the Rust
operation alive and carry one unconsumed chunk across the language boundary:

~~~swift
let cancellation = AukiCancellation()
let download = try await data.startDownload(
    dataId: recordID,
    options: AukiTransferOptions(maxChunkBytes: 1024 * 1024),
    cancellation: cancellation
)
do {
    while let chunk = try await download.next() {
        try await destination.write(chunk)
    }
    try await download.close()
} catch {
    let operationError = error
    do { try await download.close() } catch { throw error }
    throw operationError
}
~~~

For multipart upload, declare the exact nonzero byte count. Read no more than
each requested maximum and push empty `Data` at EOF:

~~~swift
let upload = try await data.startUpload(
    target: .named(name: "report", dataType: "my-app.report.v1"),
    size: fileSize,
    options: AukiTransferOptions(maxChunkBytes: 1024 * 1024),
    cancellation: cancellation
)
do {
    while let maximum = try await upload.nextMaximum() {
        let chunk = try await source.read(upToCount: Int(maximum))
        try await upload.push(bytes: chunk ?? Data())
    }
    let saved = try await upload.result()
    try await upload.close()
} catch {
    let operationError = error
    do { try await upload.close() } catch { throw error }
    throw operationError
}
~~~

Call `cancel()` to stop an operation, then await `close()`. Upload close waits
for bounded multipart abort cleanup even if the Swift task that was observing
the transfer was cancelled. Close data clients and peers before closing the
shared session; session close is independent of each data client's cleanup.

Domain data failures preserve `kind`, optional HTTP `status`, and optional
authentication `authKind`. Permission statuses such as 403 remain distinct.
Errors omit response bodies, credentials, URLs, and application bytes.

## Submit and inspect DMS jobs

The same User or imported ZITADEL session can create a Domain-scoped jobs
client. This does not start a peer or a worker, and it does not poll job state:

~~~swift
let jobs = try session.jobs(domainId: selectedDomainID)
let spec = AukiJobSpec(
    label: "map update",
    tasks: [AukiJobTaskSpec(
        label: "reconstruct",
        stage: "reconstruct",
        capability: "com.example.private/reconstruct/v1",
        mode: .dedicated,
        inputsCids: [inputID]
    )]
)

do {
    let estimate = try await jobs.estimate(spec)
    showEstimatedCredits(estimate.total) // Decimal text; do not round through Double.
    let jobID = try await jobs.submit(spec)
    let details = try await jobs.get(jobID)
    try await jobs.close()
} catch {
    try? await jobs.close()
    throw error
}
~~~

Task dependencies use stage names in `AukiJobEdge`. Capability strings are
passed through to DMS, including custom dedicated capabilities. `list` returns
one page and an opaque `nextCursor`; pass that cursor into a new query to fetch
another page. Use `AukiCancellation` for caller cancellation and always await
`close()` before closing the shared session.

A `submissionUncertain` failure means DMS may have accepted the job. Inspect
existing jobs before deciding what to do; automatic resubmission can duplicate
work and charges. Jobs errors preserve the stable error `code`, HTTP status,
size limit, and the redacted source category for ambiguous submissions.

See the [jobs reference](../../../../docs/reference/jobs.md) for required write
authority, worker availability, provider limitations and binding compatibility.
