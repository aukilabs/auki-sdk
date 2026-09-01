# Auki SDK for Swift

This is the thin Apple-platform binding for the Rust-owned `AukiPeer`
runtime. It exposes User login, explicit Domain selection, stable peer
identity bytes, default relay-backed startup, route inspection, status, and
ordered shutdown.

Swift owns platform policy such as Keychain storage and foreground/background
transitions. It does not implement authentication, relay booking, libp2p, or
protocol framing.

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
