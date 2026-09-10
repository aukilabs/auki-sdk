# Auki networking for Swift

Use `AukiUserSession` and `AukiPeer` to connect your iOS app to the Auki network.
The local Swift package requires Swift 6, iOS 17+, Xcode, and Rust 1.89+.

Build from the SDK repository root:

~~~sh
rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios
bash core/bindings/swift/auki-sdk-swift/build-xcframework.sh
~~~

Add this directory's local Swift package to your app. The build includes
experimental protocol bindings. Register the handlers you want to use.

Compile the SDK and your Rust adapter into one framework, as in
[Swift Echo](../../../examples/portable-echo/swift/README.md).
See [custom protocols](../../../../docs/how-to/protocols.md).
