# Changelog - AukiCameraStreamer

Append-only changelog for the native iOS camera streamer example.

Latest entry on top.

---

### Nils's codex · May 27, HKT, 2026

**Camera stream fanout now isolates failed streams.** The fanout snapshots active streams per frame, keeps delivering to healthy stream ids when one sink push fails, removes failed stream ids from the active set, and includes a regression test for failed-stream pruning.

Checks: `xcodebuild test -project examples/ios/AukiCameraStreamer/AukiCameraStreamer.xcodeproj -scheme AukiCameraStreamer -destination 'platform=iOS Simulator,name=iPhone 15,OS=17.5' -derivedDataPath /tmp/auki-camera-streamer-derived-data -only-testing:AukiCameraStreamerTests/CameraStreamFanoutTests/testFailingStreamDoesNotBlockHealthyStreamsAndIsRemoved`.

### Nils's codex · May 27, HKT, 2026

**Camera frame codec and stream fanout added.** The iOS streamer now has a SwiftProtobuf-backed `CameraFrameCodec` that serializes JPEG bytes into `Auki_Camera_CameraFrame.frame`, a hardware-free `CameraStreamFanout` actor for accepted stream ids, and tests proving encoded payload decoding plus stream removal. The XcodeGen spec and checked-in Xcode project now link SwiftProtobuf directly for the app/test targets that call generated protobuf serialization APIs.

Checks: `xcodebuild test -project examples/ios/AukiCameraStreamer/AukiCameraStreamer.xcodeproj -scheme AukiCameraStreamer -destination 'platform=iOS Simulator,name=iPhone 15,OS=17.5' -derivedDataPath /tmp/auki-camera-streamer-derived-data`.

### Nils's codex · May 27, HKT, 2026

**Native iOS camera streamer shell added.** The SwiftUI app skeleton defines cluster/discovery controls, logging and streaming toggles, camera descriptor defaults, an XcodeGen project for generated Swift SDK packages, and a unit test for stable camera sensor/frame ids.

Checks: Swift binding/proto generation commands; `xcodegen generate --spec examples/ios/AukiCameraStreamer/project.yml`; `git diff --check`. `xcodebuild test -project examples/ios/AukiCameraStreamer/AukiCameraStreamer.xcodeproj -scheme AukiCameraStreamer -destination 'platform=iOS Simulator,name=iPhone 15'` was attempted but did not reach Swift compilation on this machine because Xcode has no latest-runtime `iPhone 15` destination; the installed `iPhone 15,OS=17.5` destination then failed while processing multiple generated UniFFI XCFramework packages that all emit `include/module.modulemap`.
