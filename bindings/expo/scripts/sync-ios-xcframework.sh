#!/usr/bin/env bash
# Build auki-sdk-swift XCFramework and copy it into ios/Frameworks for the Expo pod.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SWIFT_CRATE="$(cd "$ROOT/../swift/auki-sdk-swift" && pwd)"
FRAMEWORKS="$ROOT/ios/Frameworks"

bash "$SWIFT_CRATE/build-xcframework.sh"

mkdir -p "$FRAMEWORKS"
rm -rf "$FRAMEWORKS/AukiSDK.xcframework"
cp -R "$SWIFT_CRATE/target-xcframework/AukiSDK.xcframework" "$FRAMEWORKS/AukiSDK.xcframework"

echo "synced AukiSDK.xcframework → $FRAMEWORKS"
