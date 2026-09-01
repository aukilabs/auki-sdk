# Swift/iOS standard protocol playground

This iOS app opts one relay-backed peer into all six standard Auki protocol
families, serves the same fixtures as the native, Web, and Python playgrounds,
and probes another peer through its pasted peer card.

The SwiftUI layer only handles User login, explicit Domain selection, copy and
paste, status, and app lifecycle. `StandardPlayground` owns the small amount of
application orchestration. Authentication, exact-peer authorization, both
Catalog wire versions, protocol bounds, transport, and cleanup remain in Rust.

The example intentionally uses an ephemeral libp2p identity. Its Peer ID stays
stable while the app process runs and changes after relaunch.

## Build

Install the Apple Rust targets and reproducible project generator once:

```sh
rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios
brew install mint
```

From this directory:

```sh
./scripts/build-bindings.sh
./scripts/generate-project.sh
```

The first command builds the generic `AukiSDK.xcframework` with every standard
protocol enabled. The app links that one Rust artifact; it does not create or
link a second protocol-specific Rust library.

Open `AukiStandardProtocolsIOS.xcodeproj`, select an iPhone simulator, and run
the `AukiStandardProtocolsIOS` scheme. Log in, select a Domain, start the peer,
then paste a native, Python, Web, or second iOS peer card and select **Probe all
six**.

## Offline simulator tests

```sh
xcodebuild \
  -project AukiStandardProtocolsIOS.xcodeproj \
  -scheme AukiStandardProtocolsIOS \
  -destination 'platform=iOS Simulator,name=iPhone 17 Pro,OS=latest' \
  CODE_SIGNING_ALLOWED=NO test
```

The tests lock peer-card parsing, fixture JSON and records, the Registry list
hash invariant, and the scalar protobuf bytes without credentials or network
access.

## Live automation

The debug app accepts the following process environment through its Xcode run
scheme or `simctl launch`:

- `AUKI_IOS_EMAIL`, `AUKI_IOS_PASSWORD`, and `AUKI_IOS_DOMAIN_ID` start a peer;
- `AUKI_IOS_REMOTE_CARD` probes that peer card after startup;
- `AUKI_IOS_NODE_NAME` changes the served Info name; and
- `AUKI_IOS_STOP_AFTER_PROBE=1` performs ordered shutdown after the probe.

Credentials are never printed. Automation can wait for
`AUKI_IOS_STANDARD_READY`, `AUKI_IOS_STANDARD_PROBE`, and
`AUKI_IOS_STANDARD_STOPPED` in the simulator log. The first iteration is
foreground-oriented; moving the app to the background requests ordered
Message receiver, Stream producer, standard endpoint, and peer shutdown.
