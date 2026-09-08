#!/usr/bin/env bash
# Build auki-sdk-swift XCFramework and copy it + UniFFI Swift into ios/ for the Expo pod.
#
# Layout mirrors @auki/expo-pnp: xcframework at ios/ root (not ios/Frameworks/), so
# CocoaPods emits [CP] Copy XCFrameworks + -l auki_sdk_swift for the app link.
#
# If Cursor/sandbox redirects cargo output, force the workspace target dir so
# build-xcframework.sh lipo paths resolve:
#   export CARGO_TARGET_DIR="$(cd auki-sdk && pwd)/target"
# Match peyote deployment floor:
#   export IPHONEOS_DEPLOYMENT_TARGET=15.1
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SWIFT_CRATE="$(cd "$ROOT/../swift/auki-sdk-swift" && pwd)"
XCF="$ROOT/ios/AukiSDK.xcframework"
SWIFT_SOURCES="$ROOT/ios/AukiSDK"

if [[ -z "${CARGO_TARGET_DIR:-}" ]]; then
  export CARGO_TARGET_DIR="$(cd "$ROOT/../.." && pwd)/target"
fi
export IPHONEOS_DEPLOYMENT_TARGET="${IPHONEOS_DEPLOYMENT_TARGET:-15.1}"

bash "$SWIFT_CRATE/build-xcframework.sh"

rm -rf "$XCF"
cp -R "$SWIFT_CRATE/target-xcframework/AukiSDK.xcframework" "$XCF"

# UniFFI high-level Swift must live in the Expo pod (same target as AukiSdkExpoModule).
rm -rf "$SWIFT_SOURCES"
mkdir -p "$SWIFT_SOURCES"
cp "$SWIFT_CRATE/Sources/AukiSDK/AukiSDK.swift" "$SWIFT_SOURCES/"
cp -R "$SWIFT_CRATE/Sources/AukiSDK/Generated" "$SWIFT_SOURCES/Generated"

echo "synced AukiSDK.xcframework → $XCF"
echo "synced UniFFI Swift → $SWIFT_SOURCES (CARGO_TARGET_DIR=$CARGO_TARGET_DIR)"
