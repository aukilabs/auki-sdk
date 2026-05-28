# AukiCameraStreamer

Native iOS producer peer for generated Auki Swift bindings. The app joins or
creates an Auki domain cluster, publishes a camera sensor/resource catalog,
writes encoded camera frames to a Sensor Log, and accepts domain camera stream
requests so Overwatch can subscribe to the same cluster.

Generate local Swift bindings and protobuf bindings from the repository root:

```bash
just generate-swift-bindings auki-network
just generate-swift-bindings auki-domain
just generate-swift-bindings auki-registry
just generate-swift-bindings auki-time
just generate-swift-bindings auki-logs
just generate-swift-bindings auki-layout
just generate-swift-bindings auki-manifests
just generate-swift-proto
xcodegen generate --spec examples/ios/AukiCameraStreamer/project.yml
```

Prepare and start Overwatch from the repository root:

```bash
just generate-javascript-bindings auki-network
just generate-javascript-bindings auki-domain
just generate-javascript-bindings auki-geometry
scripts/generate-javascript-proto.sh
node examples/overwatch/scripts/stage-sdk.mjs
npm --prefix examples/overwatch install
npm --prefix examples/overwatch run dev
```

`bindings/` is ignored, so the Swift packages above must exist locally before
opening or testing the Xcode project. Regenerate the project after adding Swift
source files because the checked-in `.xcodeproj` has explicit source membership.

Run tests:

```bash
xcodebuild test -project examples/ios/AukiCameraStreamer/AukiCameraStreamer.xcodeproj -scheme AukiCameraStreamer -destination 'platform=iOS Simulator,name=iPhone 15,OS=17.5' -derivedDataPath /tmp/auki-camera-streamer-derived-data
```

Run the app with a Discovery URL reachable from the device or simulator. The
wallet seed is stored in the iOS keychain, so the local peer id remains stable
across launches until the app/keychain item is removed.

The producer uses SDK-owned Discovery-signaled WebRTC data channels. It does not
open a native `webrtc-direct` listen address; the iOS and browser peers exchange
offers, answers, and ICE candidates through Discovery's `/signals` endpoints.

Starting the producer requests iOS camera permission, joins or creates the Auki
domain cluster, then starts an AVFoundation back-camera capture loop. Captured
BGRA camera buffers are converted to JPEG, timestamped from the SDK session
clock, logged as generated `CameraFrame` bytes, and streamed to accepted domain
camera stream consumers. The app also renders the latest local JPEG in the
Preview section so operators can confirm the camera feed without subscribing
from Overwatch.

Manual E2E flow:

1. Start Discovery, for example from `/Users/jb/Developer/Aukilabs/repos/discovery` with `cargo run -- --addr 0.0.0.0:8080`.
2. Start Overwatch with `just overwatch`.
3. Join or create a cluster in Overwatch, for example `ios-camera`.
4. Launch the iOS app on device.
5. Set the same Discovery URL Overwatch uses. On a physical phone, use the Mac's LAN IP, not `127.0.0.1`.
6. Start the app with the same cluster name.
7. Select the iOS camera sensor in Overwatch.
8. Confirm the preview updates from the iOS stream.
