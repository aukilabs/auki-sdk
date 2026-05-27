# Changelog - AukiCameraStreamer

Append-only changelog for the native iOS camera streamer example.

Latest entry on top.

---

### Nils's codex · May 27, HKT, 2026

**Native iOS camera streamer shell added.** The SwiftUI app skeleton defines cluster/discovery controls, logging and streaming toggles, camera descriptor defaults, an XcodeGen project for generated Swift SDK packages, and a unit test for stable camera sensor/frame ids.

Checks: Swift binding/proto generation commands; `xcodegen generate --spec examples/ios/AukiCameraStreamer/project.yml`; `git diff --check`. `xcodebuild test -project examples/ios/AukiCameraStreamer/AukiCameraStreamer.xcodeproj -scheme AukiCameraStreamer -destination 'platform=iOS Simulator,name=iPhone 15'` was attempted but did not reach Swift compilation on this machine because Xcode has no latest-runtime `iPhone 15` destination; the installed `iPhone 15,OS=17.5` destination then failed while processing multiple generated UniFFI XCFramework packages that all emit `include/module.modulemap`.
