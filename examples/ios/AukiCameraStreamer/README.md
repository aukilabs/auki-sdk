# AukiCameraStreamer

Native iOS producer peer shell for generated Auki Swift bindings.

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

Run tests:

```bash
xcodebuild test -project examples/ios/AukiCameraStreamer/AukiCameraStreamer.xcodeproj -scheme AukiCameraStreamer -destination 'platform=iOS Simulator,name=iPhone 15'
```

This task only adds the app shell. SDK session setup, AVFoundation capture, protobuf frame encoding, log writes, and camera stream fanout come in later tasks.
