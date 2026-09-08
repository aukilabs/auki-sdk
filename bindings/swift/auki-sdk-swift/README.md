# Auki SDK for Swift

This is the thin Apple-platform binding for the Rust-owned `AukiPeer`
runtime. It exposes User login, explicit Domain selection, stable peer
identity bytes, default relay-backed startup, route inspection, status, and
ordered shutdown.

DDS discovery is opt-in through `startPeerWithDiscovery`. Choose
`.discoverOnly` to look up peers without publishing, or
`.discoverAndAdvertise` to maintain a short-lived advertisement. The returned
peer exposes async `discover()` and `discoverProtocol(protocolId:)`; candidates
are untrusted route hints and exact dialing still verifies the Peer ID and
Domain in Rust.

Swift owns platform policy such as Keychain storage and foreground/background
transitions. It does not implement authentication, relay booking, libp2p, or
protocol framing.

## ZITADEL public-client handoff

The host owns PKCE login and secure storage. After login, stop other refresh
owners and import the five-field result. `issuer`/`clientId` are trusted app
configuration, not values inferred from an unverified token. Import returns an
`AukiSession` synchronously, without network I/O; keep it in app state before
starting a peer so that later failures do not lose a rotated token. The app supplies
a known Domain ID; ZITADEL v1 does not support `accessibleDomains()`.

```swift
let credentials = try AukiZitadelCredentials(
    accessToken: accessToken, refreshToken: refreshToken,
    clientId: clientId, issuer: issuer,
    accessTokenExpiresAt: accessTokenExpiresAt // RFC 3339 String?, nil if unknown
)
let session = try AukiSession.importZitadelDev(credentials: credentials, store: store)
// Retain session BEFORE this await. Store implements AukiZitadelSessionStore.
let peer = try await session.startPeer(domainId: selectedDomainId, identity: identity)
// Observe try await peer.waitStopped() for terminal failure.
try await peer.shutdown()
await session.close()
// Only after completed close: clear your durable credentials.
```

`importZitadelWithEnvironment(apiBaseUrl:ddsBaseUrl:dmsBaseUrl:credentials:store:)`
supports exact service bases. Existing password login and peer methods remain.

Implement `AukiZitadelSessionStore.save(credentials:) async throws`. Atomically
persist the entire snapshot (access/refresh tokens, client, issuer, expiry) to
your secure store **before returning**. Read token strings explicitly through
`exposeAccessToken()` and `exposeRefreshToken()`; the credential object has no
public stored token fields and diagnostics are redacted. Never log the extracted
strings. Prefer throwing `AukiPersistenceError.Failed` on storage failure; even
unexpected Swift error types are mapped to a redacted persistence failure.

On success **or error**, all writes started by this invocation must have settled.
Do not launch detached writes or reenter the same session from the callback.
An event announcing new tokens is not persistence acknowledgement. Rust awaits
the Swift callback and retains the replacement before invoking it. A rejected
save retries the **same generation** on the next session operation, with no
DDS admission or further rotation until acknowledged.

`close()` fences new session work and drains outstanding host storage. If its
observing Swift Task was cancelled, await `close()` again before clearing storage.
A never-settling host write prevents safe completed logout. Stop owned peers
separately; session close is not immediate remote-token revocation.

Auth operation/startup errors are `AukiSdkError.Authentication(kind:)` with
`AukiAuthFailureKind`. Handle `.authenticationRequired` with a fresh login,
`.configuration` with a configuration fix, and `.authorizationDenied` by selecting
a readable Domain. Retry `.persistence`/`.transient` using the retained handle.
`.closed`/`.cancelled` describe lifecycle termination. Peer terminal auth failures
also appear as `.failedAuthentication(kind:)` status and the same typed
`waitStopped()` error. Other peer errors retain the existing `Operation` boundary.

Expiry is RFC 3339 text, emitted as UTC `Z` with nanoseconds preserved; `nil`
means unknown. Keep it as text for durable round trips, not a floating-point
`Date`. Other existing timestamps remain RFC 3339 (`+00:00` is also UTC). Human
subjects are exact strings, not UUIDs: do not trim or normalize case/Unicode.
If comparing identifiers in Swift, compare UTF-8 bytes where canonical Unicode
equivalence would otherwise merge different identities.

See [binding runtime tests](../../../docs/zitadel-binding-tests.md) for the actual
generated Swift callback proof (success/rejection, cancellation, close, restart).

The [portable echo iOS app](../../../examples/portable-echo/swift/README.md)
shows the product-protocol shape. The
[standard protocol iOS app](../../../examples/standard-protocols/swift/README.md)
mounts and probes Info, Catalog, Registry, Blob, Message, and Stream. Both use
ephemeral identities. Persistence is optional: store
`AukiPeerIdentity.encoded()` only when the application requires a stable Peer
ID.

## Build

```bash
rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios
./bindings/swift/auki-sdk-swift/build-xcframework.sh
```

The XCFramework and generated Swift glue are intentionally not committed. The
script places both where the local `Package.swift` expects them, so an Xcode
project can depend on this directory immediately after the build. The package
also propagates the static library's Apple linker requirements.

The default deployment target is iOS 17.0; set `IPHONEOS_DEPLOYMENT_TARGET`
before running the script to override it.

The package links `SystemConfiguration.framework`, `CoreFoundation.framework`,
and `libiconv` for the final Apple target.

The build script enables the complete standard-protocol bundle. Individual
Rust features remain available for smaller custom artifacts. A custom Rust
protocol must produce one umbrella artifact that contains both this facade and
its protocol adapter; linking two separate Rust XCFrameworks would create
incompatible UniFFI object runtimes.
