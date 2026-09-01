#!/usr/bin/env bash
# Build the relay-backed Auki SDK XCFramework and generated Swift bindings.
#
# Prerequisites:
#   rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios
set -euo pipefail

CRATE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "$CRATE_DIR/../../.." && pwd)"
LIB_NAME="auki_sdk_swift"
OUT="$CRATE_DIR/target-xcframework"
BINDINGS="$OUT/bindings"
SWIFT_OUT="$CRATE_DIR/Sources/AukiSDK/Generated"

# Keep Rust object metadata aligned with the example app and declared support
# floor. Callers may override this before invoking the script.
export IPHONEOS_DEPLOYMENT_TARGET="${IPHONEOS_DEPLOYMENT_TARGET:-17.0}"

rm -rf "$OUT" "$SWIFT_OUT"
mkdir -p "$BINDINGS" "$SWIFT_OUT"
cd "$WORKSPACE_ROOT"

for TARGET in aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios; do
  cargo build --locked --release -p auki-sdk-swift --target "$TARGET"
done

DEVICE_LIB="target/aarch64-apple-ios/release/lib${LIB_NAME}.a"
SIM_FAT="$OUT/lib${LIB_NAME}-sim.a"
lipo -create \
  "target/aarch64-apple-ios-sim/release/lib${LIB_NAME}.a" \
  "target/x86_64-apple-ios/release/lib${LIB_NAME}.a" \
  -output "$SIM_FAT"

cargo run --locked --release --features cli -p auki-sdk-swift --bin uniffi-bindgen -- generate \
  --library "$DEVICE_LIB" \
  --language swift \
  --out-dir "$BINDINGS"

{
  for modulemap in "$BINDINGS"/*FFI.modulemap; do
    [ -f "$modulemap" ] || continue
    cat "$modulemap"
    echo
  done
} > "$BINDINGS/module.modulemap"
find "$BINDINGS" -name "*FFI.modulemap" -delete

for swift_file in "$BINDINGS"/*.swift; do
  [ -f "$swift_file" ] || continue
  mv "$swift_file" "$SWIFT_OUT/"
done

xcodebuild -create-xcframework \
  -library "$DEVICE_LIB" -headers "$BINDINGS" \
  -library "$SIM_FAT" -headers "$BINDINGS" \
  -output "$OUT/AukiSDK.xcframework"

# Fail here, before an application build, if generated Swift and its exact FFI
# header/module map no longer agree.
swiftc \
  -typecheck \
  -I "$BINDINGS" \
  -Xcc "-fmodule-map-file=$BINDINGS/module.modulemap" \
  "$SWIFT_OUT"/*.swift
plutil -lint "$OUT/AukiSDK.xcframework/Info.plist"

echo "XCFramework: $OUT/AukiSDK.xcframework"
echo "Swift package: $CRATE_DIR/Package.swift"
