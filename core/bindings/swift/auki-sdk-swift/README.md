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

Run the generated Swift API against the offline loopback fixture with:

~~~sh
core/bindings/swift/auki-sdk-swift/run-domain-data-bindings-test.sh
~~~

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

An imported ZITADEL session supports data access for a known Domain ID and uses
the same awaited credential store as peer authentication. The released API/DDS
contracts do not expose a safe imported-session Domain listing for every human
role, so `domains().list` remains unsupported for imported sessions. Keep the
session on
a `.DomainData(... authKind: .persistence ...)` error and retry after secure
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
