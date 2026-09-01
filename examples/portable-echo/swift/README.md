# Swift/iOS portable echo

This is the smallest iOS application built on the shared Rust Auki peer and
portable echo protocol. It logs in a User, lists accessible Domains, starts a
relay-backed peer, and exchanges typed peer cards and echo messages with the
native example.

The example intentionally uses an **ephemeral libp2p identity**. The Peer ID is
stable while the app process is running and changes after relaunch. Production
applications that require a durable Peer ID can persist
`AukiPeerIdentity.encoded()` in their platform storage without changing the
peer or protocol APIs.

## Build

Install the Apple Rust targets and project generator once:

```sh
rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios
brew install mint
```

From this directory:

```sh
./scripts/build-bindings.sh
./scripts/generate-project.sh
```

The first script builds one umbrella `AukiPortableEcho.xcframework` containing
both the generic peer facade and echo adapter. Do not link a second Auki Rust
XCFramework into the same app.

Open `AukiPortableEchoIOS.xcodeproj`, select an iPhone simulator, and run the
`AukiPortableEchoIOS` scheme. Log in, select a Domain, start the peer, then copy
and paste whole peer-card JSON between this app and another runtime.

The first iteration is foreground-oriented. Moving the app to the background
requests ordered echo and peer shutdown, but reliable long-running background
networking is outside this example's scope.

## Simulator checks

```sh
xcodebuild \
  -project AukiPortableEchoIOS.xcodeproj \
  -scheme AukiPortableEchoIOS \
  -destination 'platform=iOS Simulator,name=iPhone 17 Pro,OS=latest' \
  CODE_SIGNING_ALLOWED=NO test
```

For repeatable live interop, the debug app also accepts `AUKI_IOS_EMAIL`,
`AUKI_IOS_PASSWORD`, `AUKI_IOS_DOMAIN_ID`, optional `AUKI_IOS_REMOTE_CARD`, and
optional `AUKI_IOS_MESSAGE` in its process environment. Credentials are never
printed. Set `AUKI_IOS_STOP_AFTER_RECEIVE=1` when a smoke run should perform
ordered echo and peer shutdown after its first inbound message.
