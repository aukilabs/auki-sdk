# AukiCameraStreamer

Native iOS producer peer for generated Auki Swift bindings. The app joins or
creates an Auki domain cluster, publishes a camera sensor/resource catalog,
writes encoded camera frames to a Sensor Log, and accepts domain camera stream
requests so Overwatch can subscribe to the same cluster.

Generate local Swift bindings and protobuf bindings from the repository root:

```bash
python3 scripts/bindings/generate_bindings.py generate swift auki-domain
python3 scripts/bindings/generate_bindings.py generate swift auki-network
python3 scripts/bindings/generate_bindings.py generate swift auki-registry
python3 scripts/bindings/generate_bindings.py generate swift auki-time
python3 scripts/bindings/generate_bindings.py generate swift auki-logs
python3 scripts/bindings/generate_bindings.py generate swift auki-layout
python3 scripts/bindings/generate_bindings.py generate swift auki-manifests
scripts/generate-swift-proto.sh
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

Starting the producer requests iOS camera permission, joins or creates the Auki
domain cluster, then starts an AVFoundation back-camera capture loop. Captured
BGRA camera buffers are converted to JPEG, timestamped from the SDK session
clock, logged as generated `CameraFrame` bytes, and streamed to accepted domain
camera stream consumers. The app also renders the latest local JPEG in the
Preview section so operators can confirm the camera feed without subscribing
from Overwatch.

Manual E2E flow:

1. Launch iOS app on device.
2. Set the same Discovery URL Overwatch uses.
3. Start the app with cluster name ios-camera.
4. Open Overwatch.
5. Join cluster ios-camera.
6. Select the iOS camera sensor.
7. Confirm the preview updates from the iOS stream.
