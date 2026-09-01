# Auki SDK for Swift

This is the thin Apple-platform binding for the Rust-owned `AukiPeer`
runtime. It exposes User login, explicit Domain selection, stable peer
identity bytes, default relay-backed startup, route inspection, status, and
ordered shutdown.

Swift owns platform policy such as Keychain storage and foreground/background
transitions. It does not implement authentication, relay booking, libp2p, or
protocol framing.

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
