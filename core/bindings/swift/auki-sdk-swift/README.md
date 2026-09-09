# Auki networking for Swift

`AukiUserSession` and `AukiPeer` expose the Rust runtime to iOS applications.
The local Swift package requires Swift 6, iOS 17+, Xcode, and Rust 1.89+.

Build from the SDK repository root:

~~~sh
rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios
bash core/bindings/swift/auki-sdk-swift/build-xcframework.sh
~~~

Add this directory's local Swift package to your app. The build script also
includes the current experimental protocol bindings; inclusion does not mount
endpoints or establish protocol stability.

For a custom protocol, compile the peer and adapter into one framework, as in
[Swift Echo](../../../examples/portable-echo/swift/README.md).
See the [networking reference](../../../../docs/reference/networking.md).
