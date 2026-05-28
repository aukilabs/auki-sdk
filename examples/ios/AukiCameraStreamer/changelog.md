# Changelog - AukiCameraStreamer

Append-only changelog for the native iOS camera streamer example.

Latest entry on top.

---

### Nils's codex · May 28, HKT, 2026

**Camera stream fanout now awaits SDK stream-entry writes.** `DomainCameraStreamSink` now calls the signaled Domain peer's async stream-push path so camera frames are backpressured by the core Swift SDK data-channel transport instead of being launched as unbounded fire-and-forget tasks. The app-facing tests also lock the signaled WebRTC chunking contract used for large frame payloads.

Checks: `xcodebuild test -project examples/ios/AukiCameraStreamer/AukiCameraStreamer.xcodeproj -scheme AukiCameraStreamer -destination 'platform=iOS Simulator,id=5FDB8F06-16CE-444F-8852-A295466B114F' -only-testing:AukiCameraStreamerTests/AukiCameraModelsTests/testSignaledWebRtcLengthPrefixedFramesAreChunkedForDataChannelTransport -only-testing:AukiCameraStreamerTests/CameraStreamFanoutTests/testDomainCameraStreamSinkUsesAsyncPushPath CODE_SIGNING_ALLOWED=NO`.

### Nils's codex · May 28, HKT, 2026

**Signaled WebRTC two-peer join regression covered.** `AukiCameraStreamerTests` now runs two `AukiSignaledWebRTCPeer` instances against an in-memory Discovery signal mailbox, proving `/auki/join/0.0.1` framed requests complete over SDK-owned signaled WebRTC data channels and preserving the existing no-answer timeout behavior.

Checks: red/green `xcodebuild test -project examples/ios/AukiCameraStreamer/AukiCameraStreamer.xcodeproj -scheme AukiCameraStreamer -destination 'platform=iOS Simulator,name=iPhone 15,OS=17.5' -derivedDataPath /tmp/auki-camera-streamer-two-peer-green CODE_SIGNING_ALLOWED=NO -only-testing:AukiCameraStreamerTests/AukiCameraModelsTests/testSignaledWebRtcFramedRequestCompletesBetweenTwoPeers`; `xcodebuild test -project examples/ios/AukiCameraStreamer/AukiCameraStreamer.xcodeproj -scheme AukiCameraStreamer -destination 'platform=iOS Simulator,name=iPhone 15,OS=17.5' -derivedDataPath /tmp/auki-camera-streamer-signaled-focused CODE_SIGNING_ALLOWED=NO -only-testing:AukiCameraStreamerTests/AukiCameraModelsTests/testSignaledWebRtcFramedRequestCompletesBetweenTwoPeers -only-testing:AukiCameraStreamerTests/AukiCameraModelsTests/testSignaledWebRtcFramedRequestTimesOutWhenPeerNeverAnswers`; `xcodebuild test -project examples/ios/AukiCameraStreamer/AukiCameraStreamer.xcodeproj -scheme AukiCameraStreamer -destination 'platform=iOS Simulator,name=iPhone 15,OS=17.5' -derivedDataPath /tmp/auki-camera-streamer-derived-data CODE_SIGNING_ALLOWED=NO`.

### Nils's codex · May 28, HKT, 2026

**Signaled WebRTC timeout regression covered.** `AukiCameraStreamerTests` now directly exercises `AukiSignaledWebRTCPeer.requestFramed` with a no-answer Discovery fake and a short operation timeout, proving the app-facing generated SDK path fails with `.timedOut` instead of hanging in `Starting`. The XcodeGen spec and checked-in project now link `AukiNetworkSignaledWebRTC` explicitly for app/test targets that import or embed the signaled transport.

Checks: red/green `xcodebuild test -project examples/ios/AukiCameraStreamer/AukiCameraStreamer.xcodeproj -scheme AukiCameraStreamer -destination 'platform=iOS Simulator,name=iPhone 15,OS=17.5' -derivedDataPath /tmp/auki-camera-streamer-timeout-green CODE_SIGNING_ALLOWED=NO -only-testing:AukiCameraStreamerTests/AukiCameraModelsTests/testSignaledWebRtcFramedRequestTimesOutWhenPeerNeverAnswers`; `xcodegen generate --spec examples/ios/AukiCameraStreamer/project.yml`; `xcodebuild test -project examples/ios/AukiCameraStreamer/AukiCameraStreamer.xcodeproj -scheme AukiCameraStreamer -destination 'platform=iOS Simulator,name=iPhone 15,OS=17.5' -derivedDataPath /tmp/auki-camera-streamer-derived-data CODE_SIGNING_ALLOWED=NO`; `plutil -lint examples/ios/AukiCameraStreamer/AukiCameraStreamer.xcodeproj/project.pbxproj`; `git diff --check`.

### Nils's codex · May 27, HKT, 2026

**Discovery-signaled WebRTC transport enabled.** `AukiCameraStreamer` now uses `AukiSignaledWebRTCDomainPeer` instead of `DomainClusterManager`'s auto-advertised `webrtc-direct` path, exposes the transport contract as `discovery-signaled-webrtc` with no listen addresses, and links the new `AukiDomainSignaledWebRTC` package product. The camera catalogs and registry entries are installed before the app joins an existing Overwatch-managed cluster, so Overwatch's first post-join catalog refresh can see the iOS camera sensor. The runbook now calls out the Discovery `/signals` path, binding-generation order, Discovery startup, and physical-device LAN URL requirement.

Checks: `xcodegen generate --spec examples/ios/AukiCameraStreamer/project.yml`; `xcodebuild test -project examples/ios/AukiCameraStreamer/AukiCameraStreamer.xcodeproj -scheme AukiCameraStreamer -destination 'platform=iOS Simulator,name=iPhone 15,OS=17.5' -derivedDataPath /tmp/auki-camera-streamer-derived-data CODE_SIGNING_ALLOWED=NO`; `git diff --check`.

### Nils's codex · May 27, HKT, 2026

**Overwatch runbook staging documented.** The manual E2E runbook now includes JavaScript binding/protobuf generation and `examples/overwatch/scripts/stage-sdk.mjs` before `npm install`, so a fresh checkout prepares the ignored `examples/overwatch/sdk-generated` file packages that Overwatch depends on.

Checks: `git diff --check`.

### Nils's codex · May 27, HKT, 2026

**Operator status and manual E2E runbook completed.** The view model now exposes the Task 7 control surface defaults, bridges the camera preview through `lastPreviewImage`, polls session status for session id, accepted stream count, logged frame count, last frame timestamp, and last error, and the SwiftUI status section shows those live values without instructional copy. The README now documents the exact binding/protobuf/project generation commands, Overwatch startup commands, and the manual device-to-Overwatch verification flow.

Checks: `xcodebuild build -project examples/ios/AukiCameraStreamer/AukiCameraStreamer.xcodeproj -scheme AukiCameraStreamer -destination 'platform=iOS Simulator,name=iPhone 15,OS=17.5' -derivedDataPath /tmp/auki-camera-streamer-build-derived-data`; `xcodebuild test -project examples/ios/AukiCameraStreamer/AukiCameraStreamer.xcodeproj -scheme AukiCameraStreamer -destination 'platform=iOS Simulator,name=iPhone 15,OS=17.5' -derivedDataPath /tmp/auki-camera-streamer-derived-data`; `git diff --check`; `plutil -lint examples/ios/AukiCameraStreamer/AukiCameraStreamer.xcodeproj/project.pbxproj`.

### Nils's codex · May 27, HKT, 2026

**Capture lifecycle races hardened.** The view model now cancels and drains in-flight frame forwarding tasks before session shutdown, rechecks startup cancellation after camera permission returns, and the capture service rolls back a partially-added camera input if video output configuration fails.

Checks: `xcodebuild build -project examples/ios/AukiCameraStreamer/AukiCameraStreamer.xcodeproj -scheme AukiCameraStreamer -destination 'platform=iOS Simulator,name=iPhone 15,OS=17.5' -derivedDataPath /tmp/auki-camera-streamer-build-derived-data`.

### Nils's codex · May 27, HKT, 2026

**AVFoundation camera capture added.** `AukiCameraStreamer` now requests camera permission, starts the generated-binding domain session before emitting frames, captures back-camera BGRA sample buffers through a serial AVFoundation queue, throttles JPEG conversion to 10 fps, timestamps frames from the session clock, updates a local SwiftUI preview image, and forwards the same `CapturedCameraFrame.jpegBytes` into the existing Sensor Log and domain stream path. The README now documents the camera permission and local preview behavior, and XcodeGen was rerun so the checked-in project includes the capture service source.

Checks: `xcodegen generate --spec examples/ios/AukiCameraStreamer/project.yml`; `xcodebuild build -project examples/ios/AukiCameraStreamer/AukiCameraStreamer.xcodeproj -scheme AukiCameraStreamer -destination 'platform=iOS Simulator,name=iPhone 15,OS=17.5' -derivedDataPath /tmp/auki-camera-streamer-build-derived-data`; `xcodebuild test -project examples/ios/AukiCameraStreamer/AukiCameraStreamer.xcodeproj -scheme AukiCameraStreamer -destination 'platform=iOS Simulator,name=iPhone 15,OS=17.5' -derivedDataPath /tmp/auki-camera-streamer-derived-data`; `git diff --check`.

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
