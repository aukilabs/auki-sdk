# Swift/iOS Camera Mesh

This foreground-only SwiftUI app can either publish the iPhone camera or view
another Camera Mesh publisher.

- Rust owns authentication, relay booking, DDS discovery, exact-peer approval,
  all six standard protocols, hashes, and ordered shutdown.
- Swift owns the UI, app lifecycle, and AVFoundation camera capture.
- The publisher retains only the newest 480×270 JPEG and serves at 5 fps.

The identity is process-scoped: relaunching the app creates a new Peer ID.
Credentials are used only for login and are not stored in Keychain. Sending the
app to the background stops capture, protocol endpoints, and the peer.

## Build and test

Install Xcode with an iOS 17 or newer SDK, the Apple Rust targets, and XcodeGen:

```sh
rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios
brew install mint
```

From this directory, generate the one umbrella binding and Xcode project:

```sh
./scripts/build-bindings.sh
./scripts/generate-project.sh
```

`AukiCameraMesh.xcframework` contains the generic SDK binding and the thin
Camera Mesh publisher bridge. Edit `project.yml`, not the ignored generated
`.xcodeproj`.

Run the offline contract and app tests:

```sh
IOS_SIM_DERIVED="$(mktemp -d "${TMPDIR%/}/auki-camera-mesh-sim.XXXXXX")"
xcodebuild \
  -project AukiCameraMeshIOS.xcodeproj \
  -scheme AukiCameraMeshIOS \
  -configuration Debug \
  -destination 'platform=iOS Simulator,name=iPhone 17 Pro,OS=latest' \
  -derivedDataPath "$IOS_SIM_DERIVED" \
  CODE_SIGNING_ALLOWED=NO \
  test
```

These tests need neither credentials nor network access. Camera capture and
live relay interoperability require a physical iPhone.

## Run on an iPhone

The simplest route is to open `AukiCameraMeshIOS.xcodeproj`, select a
Development Team in **Signing & Capabilities**, choose an unlocked and trusted
iPhone, and press Run.

For a repeatable command-line install, inspect the destinations and use the
Team ID selected in Xcode:

```sh
xcodebuild \
  -project AukiCameraMeshIOS.xcodeproj \
  -scheme AukiCameraMeshIOS \
  -showdestinations

IOS_DEVICE_UDID='<physical-device-id>'
IOS_TEAM_ID='<Apple Developer Team ID>'
IOS_DEVICE_DERIVED="$(mktemp -d "${TMPDIR%/}/auki-camera-mesh-phone.XXXXXX")"

xcodebuild \
  -project AukiCameraMeshIOS.xcodeproj \
  -scheme AukiCameraMeshIOS \
  -configuration Debug \
  -destination "platform=iOS,id=$IOS_DEVICE_UDID" \
  -derivedDataPath "$IOS_DEVICE_DERIVED" \
  -allowProvisioningUpdates \
  -allowProvisioningDeviceRegistration \
  DEVELOPMENT_TEAM="$IOS_TEAM_ID" \
  CODE_SIGN_STYLE=Automatic \
  build

IOS_APP_PATH="$IOS_DEVICE_DERIVED/Build/Products/Debug-iphoneos/AukiCameraMeshIOS.app"
xcrun devicectl device install app --device "$IOS_DEVICE_UDID" "$IOS_APP_PATH"
xcrun devicectl device process launch \
  --device "$IOS_DEVICE_UDID" \
  --terminate-existing \
  com.aukilabs.examples.CameraMesh
```

## Use the viewer

Log in, choose the publisher's Domain, select **Viewer**, and start. The viewer
uses DDS discovery without advertising itself; a complete peer card is the
fallback.

The first connection should return `approval_required`. Compare the complete
Peer ID shown by the app with the publisher's pending request, approve that
exact ID, then retry. Verify advancing JPEGs, pause, resume, and a snapshot with
a SHA-256 hash. The [Web](../web/README.md) and
[native](../native/README.md) guides provide publishers.

## Use the publisher

Log in, choose a Domain, select **Publisher**, and start. Grant camera access.
The app previews the back camera, books TCP and WSS relay routes, advertises the
Stream protocol through DDS, and displays its complete peer card.

Connect from a Web or native viewer. Before approval, the viewer must receive no
camera metadata or frames. The phone displays the requesting Peer ID in
**Pending viewers**; verify the complete value and approve it. Approval lasts
only for this publisher process and can be revoked in the app.

For the native one-command acceptance flow, start a native viewer and send the
following JSON line after replacing `target` with the phone's complete card:

```text
{"command":"exercise_live","id":"iphone-live","target":<PHONE_CARD>,"requestId":"iphone-snapshot"}
```

The first attempt triggers approval and fails. Approve its exact Peer ID on the
phone, then send the same line again. A successful `exercise_live_result`
proves one continuous subscription received two frames, became quiet after
Pause, advanced after Resume, and fetched a verified snapshot.

The Web viewer proves the WSS direction separately. Discover or paste the phone
card, trigger and grant approval, then verify frames, Pause, Resume, and
Snapshot. Finally background or stop the iOS app and confirm the viewer loses
the stream cleanly.

No background camera, audio, Bonjour, or local-network entitlement is part of
this example.

## Debug automation

Viewer automation remains available through `AUKI_IOS_EMAIL`,
`AUKI_IOS_PASSWORD`, `AUKI_IOS_DOMAIN_ID`, and `AUKI_IOS_REMOTE_CARD` in the
Run scheme. `AUKI_IOS_RUN_ACCEPTANCE=1` pauses, resumes, and requests a snapshot
after two frames. Automation always selects the viewer role and never prints
credentials.
