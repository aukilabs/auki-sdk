# Portable Echo in Swift

Requires macOS, Xcode, Swift 6, Rust 1.89+, and an iOS 17+ device or simulator.
Use the development User/Domain from the
[networking tutorial](../../../../docs/tutorials/first-peer.md).

From the SDK repository root:

~~~sh
rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios
brew install mint
cd core/examples/portable-echo/swift
./scripts/build-bindings.sh
./scripts/generate-project.sh
~~~

Open `AukiPortableEchoIOS.xcodeproj` in Xcode and run
`AukiPortableEchoIOS`. Log in, select the same Domain as another running Echo
peer, start, refresh discovered peers, select one, and send a message. Check
that the response matches what you sent.

Each start creates a new Peer ID. The app shuts down the peer when it goes
into the background. It compiles the SDK and Echo code into one framework. See
[peer lifetime](../../../../docs/how-to/lifecycle.md) and
[custom protocols](../../../../docs/how-to/protocols.md).
