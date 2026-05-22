# AukiNetworkTestApp

Minimal iOS host app for generated Auki Swift bindings.

The app imports generated SDK packages:

- `auki_network`
- `AukiProto`

Before opening the project, generate local bindings from the repository root:

```bash
just generate-swift-proto
just generate-swift-bindings auki-network
```

Then build the app:

```bash
xcodebuild \
  -project examples/ios/AukiNetworkTestApp/AukiNetworkTestApp.xcodeproj \
  -scheme AukiNetworkTestApp \
  -destination 'generic/platform=iOS Simulator' \
  build
```

This app is a host/test harness only. SDK networking behavior lives in Rust crates and generated Swift bindings.
