# Changelog - AukiCameraStreamer

Append-only changelog for the native iOS camera streamer example.

Latest entry on top.

---

### Nils's codex · May 27, HKT, 2026

**Session start, stop, and seed races hardened.** The view model now cancels in-flight startup cleanly when the user stops before bootstrap completes, session shutdown still flushes logs and shuts down the domain manager if finishing a stream fails, and concurrent first keychain seed creation re-reads the persisted seed on duplicate writes.

Checks: `xcodebuild test -project examples/ios/AukiCameraStreamer/AukiCameraStreamer.xcodeproj -scheme AukiCameraStreamer -destination 'platform=iOS Simulator,name=iPhone 15,OS=17.5' -derivedDataPath /tmp/auki-camera-streamer-derived-data`; `git diff --check`.

### Nils's codex · May 27, HKT, 2026

**Camera Sensor Logs are scoped per session.** `AukiCameraSession` now opens `BytesLog` under a session-specific camera log root so restarting the app with a new session id cannot reuse an older `log_manifest.json`; unit coverage locks the log-root path contract.

Checks: `xcodebuild test -project examples/ios/AukiCameraStreamer/AukiCameraStreamer.xcodeproj -scheme AukiCameraStreamer -destination 'platform=iOS Simulator,name=iPhone 15,OS=17.5' -derivedDataPath /tmp/auki-camera-streamer-derived-data`.

### Nils's codex · May 27, HKT, 2026

**Native iOS camera session coordination added.** The streamer now stores a stable 32-byte wallet seed in the iOS keychain, derives the local peer id before bootstrapping `DomainClusterManager` with auto-advertised WebRTC-direct listen addresses, creates a session clock, writes clock/frame/sensor registry entries, opens a retained Sensor Log, publishes static sensor/resource/registry catalogs, accepts matching camera stream requests, and declines unknown or unavailable sensors. Camera frames are encoded once per captured frame, with the same `CameraFrame` bytes appended to the log and pushed to all accepted domain streams. The view model now starts/stops the real session and exposes `handleCapturedFrame(_:)` for the capture task. XcodeGen was rerun so the checked-in project includes the new Swift sources.

Checks: `xcodebuild test -project examples/ios/AukiCameraStreamer/AukiCameraStreamer.xcodeproj -scheme AukiCameraStreamer -destination 'platform=iOS Simulator,name=iPhone 15,OS=17.5' -derivedDataPath /tmp/auki-camera-streamer-derived-data`; `git diff --check`.

### Nils's codex · May 27, HKT, 2026

**Camera stream fanout now isolates failed streams.** The fanout snapshots active streams per frame, keeps delivering to healthy stream ids when one sink push fails, removes failed stream ids from the active set, and includes a regression test for failed-stream pruning.

Checks: `xcodebuild test -project examples/ios/AukiCameraStreamer/AukiCameraStreamer.xcodeproj -scheme AukiCameraStreamer -destination 'platform=iOS Simulator,name=iPhone 15,OS=17.5' -derivedDataPath /tmp/auki-camera-streamer-derived-data -only-testing:AukiCameraStreamerTests/CameraStreamFanoutTests/testFailingStreamDoesNotBlockHealthyStreamsAndIsRemoved`.

### Nils's codex · May 27, HKT, 2026

**Camera frame codec and stream fanout added.** The iOS streamer now has a SwiftProtobuf-backed `CameraFrameCodec` that serializes JPEG bytes into `Auki_Camera_CameraFrame.frame`, a hardware-free `CameraStreamFanout` actor for accepted stream ids, and tests proving encoded payload decoding plus stream removal. The XcodeGen spec and checked-in Xcode project now link SwiftProtobuf directly for the app/test targets that call generated protobuf serialization APIs.

Checks: `xcodebuild test -project examples/ios/AukiCameraStreamer/AukiCameraStreamer.xcodeproj -scheme AukiCameraStreamer -destination 'platform=iOS Simulator,name=iPhone 15,OS=17.5' -derivedDataPath /tmp/auki-camera-streamer-derived-data`.

### Nils's codex · May 27, HKT, 2026

**Native iOS camera streamer shell added.** The SwiftUI app skeleton defines cluster/discovery controls, logging and streaming toggles, camera descriptor defaults, an XcodeGen project for generated Swift SDK packages, and a unit test for stable camera sensor/frame ids.

Checks: Swift binding/proto generation commands; `xcodegen generate --spec examples/ios/AukiCameraStreamer/project.yml`; `git diff --check`. `xcodebuild test -project examples/ios/AukiCameraStreamer/AukiCameraStreamer.xcodeproj -scheme AukiCameraStreamer -destination 'platform=iOS Simulator,name=iPhone 15'` was attempted but did not reach Swift compilation on this machine because Xcode has no latest-runtime `iPhone 15` destination; the installed `iPhone 15,OS=17.5` destination then failed while processing multiple generated UniFFI XCFramework packages that all emit `include/module.modulemap`.
