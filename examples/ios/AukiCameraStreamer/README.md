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
scripts/generate-swift-proto.sh
xcodegen generate --spec examples/ios/AukiCameraStreamer/project.yml
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
across launches until the app/keychain item is removed. Task 6 wires the
AVFoundation capture loop into `handleCapturedFrame(_:)`; the session and
domain/log/stream coordination are already in place.
