# Swift/iOS standard protocol playground

This iOS app opts one relay-backed peer into all six standard Auki protocol
families, serves the same fixtures as the native, Web, and Python playgrounds,
and probes another discovered peer or an explicitly pasted fallback card.

Before startup the app explicitly chooses **discover and advertise** (the
default) or **discover only**. Once running, it can list every current DDS
candidate or filter by one exact mounted protocol, select a candidate, and
probe it without exchanging cards. Discovery results remain untrusted until
the exact protocol connection authenticates the Peer ID and Domain; card paste
is retained as a fallback for private peers.

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
six**. Alternatively, select **Discover**, choose the peer, and select **Probe
selected peer**.

The same flow runs on a physical iPhone. The live physical-iPhone/native-Rust
gate uses **discover and advertise** on both peers. Native discovers the iPhone
through DDS; the iPhone selects native through the explicit peer-card fallback;
and all six protocol families pass in both directions. The current dev relay
publishes `/dns4` routes, so this gate does not yet cover IPv6-only/NAT64
networks or live iOS-originated DDS selection.

## Offline simulator tests

```sh
xcodebuild \
  -project AukiStandardProtocolsIOS.xcodeproj \
  -scheme AukiStandardProtocolsIOS \
  -destination 'platform=iOS Simulator,name=iPhone 17 Pro,OS=latest' \
  CODE_SIGNING_ALLOWED=NO test
```

These are offline unit tests. They lock peer-card parsing and native route
selection, fixture JSON and records, the Registry list hash invariant, and the
scalar protobuf bytes without credentials or network access. They do not prove
live Swift interoperability.

## Live automation

The debug app accepts the following process environment through its Xcode run
scheme or `simctl launch`:

- `AUKI_IOS_EMAIL`, `AUKI_IOS_PASSWORD`, and `AUKI_IOS_DOMAIN_ID` start a peer;
- `AUKI_IOS_REMOTE_CARD` probes that peer card after startup;
- `AUKI_IOS_NODE_NAME` changes the served Info name; and
- `AUKI_IOS_STOP_AFTER_PROBE=1` performs ordered shutdown after the probe.

Credentials are never printed. Automation can wait for
`AUKI_IOS_STANDARD_READY` and `AUKI_IOS_STANDARD_PROBE` in the simulator log.
`AUKI_IOS_STANDARD_STOPPED` is printed after an explicit stop, a background
transition, or an automated probe when `AUKI_IOS_STOP_AFTER_PROBE=1` is set.
Moving the app to the background during startup invalidates the operation and
shuts down any provisional peer. After startup, it requests ordered Message
receiver, Stream producer, standard endpoint, and peer shutdown.
