# AukiCameraStreamer

Native iOS producer peer shell for generated Auki Swift bindings.

Generate local Swift bindings and protobuf bindings from the repository root:

```bash
python3 scripts/bindings/generate_bindings.py generate swift auki-network
python3 scripts/bindings/generate_bindings.py generate swift auki-domain
python3 scripts/bindings/generate_bindings.py generate swift auki-registry
python3 scripts/bindings/generate_bindings.py generate swift auki-time
python3 scripts/bindings/generate_bindings.py generate swift auki-logs
python3 scripts/bindings/generate_bindings.py generate swift auki-layout
python3 scripts/bindings/generate_bindings.py generate swift auki-manifests
scripts/generate-swift-proto.sh
xcodegen generate --spec examples/ios/AukiCameraStreamer/project.yml
```

Run tests:

```bash
xcodebuild test -project examples/ios/AukiCameraStreamer/AukiCameraStreamer.xcodeproj -scheme AukiCameraStreamer -destination 'platform=iOS Simulator,name=iPhone 15'
```

This task only adds the app shell. SDK session setup, AVFoundation capture, protobuf frame encoding, log writes, and camera stream fanout come in later tasks.
