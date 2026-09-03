# Swift/iOS Camera Mesh viewer

This foreground-only SwiftUI app consumes a Camera Mesh publisher over the
same Rust-owned Info, Catalog, Registry, Stream, Message, and Blob protocols as
the Web, native Rust, and Python examples. It discovers Stream publishers or
accepts a complete peer card, authenticates the exact Peer ID and Domain,
renders JPEG frames, sends pause/resume, and fetches a SHA-256-verified
snapshot. It does not publish the iPhone camera.

The app uses a process-scoped `AukiPeerIdentity`; its Peer ID changes after the
app is relaunched. Credentials are held only long enough to log in and are not
stored in Keychain. Sending the app to the background performs ordered viewer
and peer shutdown, so return to the foreground and log in again before another
test.

## Build and test

Install Xcode with an iOS 17 or newer SDK, the Apple Rust targets, and XcodeGen:

```sh
rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios
brew install mint
```

Then, from this directory:

```sh
./scripts/build-bindings.sh
./scripts/generate-project.sh
```

The binding script builds one umbrella `AukiSDK.xcframework` with the six
standard protocols. The project is generated from `project.yml`; edit that
file rather than the ignored `.xcodeproj`.

Run the offline app and contract tests with temporary derived data:

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

These tests require neither credentials nor network access. They do not prove
live relay reachability. Before signing, the generic arm64 app can also be
compiled with:

```sh
IOS_ARM64_DERIVED="$(mktemp -d "${TMPDIR%/}/auki-camera-mesh-arm64.XXXXXX")"
xcodebuild \
  -project AukiCameraMeshIOS.xcodeproj \
  -scheme AukiCameraMeshIOS \
  -configuration Debug \
  -destination 'generic/platform=iOS' \
  -derivedDataPath "$IOS_ARM64_DERIVED" \
  CODE_SIGNING_ALLOWED=NO \
  build
```

## Run on an iPhone

The simplest route is to open `AukiCameraMeshIOS.xcodeproj`, choose the app
target's **Signing & Capabilities** tab and a Development Team, select an
unlocked and trusted iPhone, and press Run. Enable Developer Mode on the phone
if Xcode requests it. If the default bundle identifier is unavailable to your
team, choose a unique `PRODUCT_BUNDLE_IDENTIFIER` in `project.yml` and
regenerate the project.

For a repeatable command-line install, first inspect the destinations and set
the Apple Developer Team ID shown by Xcode for the intended account. A signing
certificate's parenthesized suffix is not necessarily its Team ID, so do not
derive this value from `security find-identity`.

```sh
xcodebuild \
  -project AukiCameraMeshIOS.xcodeproj \
  -scheme AukiCameraMeshIOS \
  -showdestinations
IOS_DEVICE_UDID="$(
  xcodebuild \
    -project AukiCameraMeshIOS.xcodeproj \
    -scheme AukiCameraMeshIOS \
    -showdestinations 2>/dev/null |
    sed -nE 's/.*platform:iOS, arch:arm64(e)?, id:([[:xdigit:]]{8}-[[:xdigit:]]{16}|[[:xdigit:]]{40}), name:.*/\2/p' |
    head -n 1
)"
IOS_TEAM_ID='<Team ID selected in Xcode>'
test -n "$IOS_DEVICE_UDID"
test "${IOS_TEAM_ID#<}" = "$IOS_TEAM_ID"
```

Build, install, and launch that exact destination:

```sh
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
test -d "$IOS_APP_PATH"
xcrun devicectl device install app --device "$IOS_DEVICE_UDID" "$IOS_APP_PATH"
xcrun devicectl device process launch \
  --device "$IOS_DEVICE_UDID" \
  --terminate-existing \
  --console \
  com.aukilabs.examples.CameraMesh
```

`--console` waits and shows the app's debug sentinels; omit it for a detached
launch. Add the Apple ID to Xcode **Settings > Accounts** first if no Apple
Development identity is available.

For debug automation, set `AUKI_IOS_EMAIL`, `AUKI_IOS_PASSWORD`,
`AUKI_IOS_DOMAIN_ID`, and `AUKI_IOS_REMOTE_CARD` in the Run scheme. Optional
`AUKI_IOS_RETRY_AFTER_APPROVAL_SECONDS` retries once after an operator has had
time to approve; `AUKI_IOS_RUN_ACCEPTANCE=1` pauses, resumes, and requests a
snapshot after two frames; and `AUKI_IOS_STOP_AFTER_SNAPSHOT=1` then stops in
order. A `devicectl` launch can pass the same values with the
`DEVICECTL_CHILD_` prefix. Credentials are never printed. Console automation
can wait for `AUKI_IOS_CAMERA_READY`, `AUKI_IOS_CAMERA_APPROVAL_REQUIRED`,
`AUKI_IOS_CAMERA_CONNECTED`, `AUKI_IOS_CAMERA_FRAME`,
`AUKI_IOS_CAMERA_SNAPSHOT`, and `AUKI_IOS_CAMERA_STOPPED`.

## Approval and physical-device QA

On the phone, log in, choose the same Domain as the publisher, and select
**Start viewer**. Use **Discover Stream publishers**, or paste only the
publisher's peer-card object into the fallback field. The first connection is
expected to report `approval_required`. Compare the complete local iOS Peer ID
with the pending ID on the publisher, approve that exact ID, then select
**Retry after approval**.

Exercise both required publisher paths:

1. Follow the [Web guide](../web/README.md), choose **Publisher**, and publish
   the synthetic source. Approve the phone in **Pending viewers**.
2. Follow the [native guide](../native/README.md) to start its deterministic
   publisher. After its `approval_required` event, send this JSON line to its
   stdin, substituting the full Peer ID shown on the phone:

   ```json
   {"command":"approve","id":"allow-phone","peerId":"<iOS Peer ID>"}
   ```

For each publisher, verify that JPEG frame count and sequence advance, Pause
halts the feed after any in-flight frame, Resume restarts it, Snapshot shows an
image and verified hash, and Disconnect returns to the publisher picker.
Finally select **Stop viewer** and repeat once by backgrounding the app; both
paths must reach ordered stop.
The Phase 3 acceptance gate is complete only after these Web-to-iPhone and
native-to-iPhone runs pass on a physical device.

This viewer needs no camera permission, Bonjour declaration, or local-network
entitlement: it captures nothing and uses the authenticated DDS tracker and
relay routes. It intentionally does not support background streaming.
