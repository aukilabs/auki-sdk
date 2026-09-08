#!/usr/bin/env bash
# Build one umbrella Apple artifact containing the generic Auki SDK facade and
# the portable echo adapter. Never link two independent Rust XCFrameworks into
# the same application.
set -euo pipefail

SWIFT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORKSPACE_ROOT="$(cd "$SWIFT_DIR/../../.." && pwd)"
LIB_NAME="auki_portable_echo_swift"
BUILD_OUT="$SWIFT_DIR/.build-bindings"
BINDINGS="$BUILD_OUT/bindings"
ARTIFACTS="$SWIFT_DIR/Package/Artifacts"
SWIFT_OUT="$SWIFT_DIR/Package/Sources/AukiPortableEcho/Generated"

export IPHONEOS_DEPLOYMENT_TARGET="${IPHONEOS_DEPLOYMENT_TARGET:-17.0}"

rm -rf "$BUILD_OUT" "$ARTIFACTS" "$SWIFT_OUT"
mkdir -p "$BINDINGS" "$ARTIFACTS" "$SWIFT_OUT"
cd "$WORKSPACE_ROOT"

for TARGET in aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios; do
  cargo build --locked --release -p auki-portable-echo-swift --target "$TARGET"
done

DEVICE_LIB="target/aarch64-apple-ios/release/lib${LIB_NAME}.a"
SIM_FAT="$BUILD_OUT/lib${LIB_NAME}-sim.a"
lipo -create \
  "target/aarch64-apple-ios-sim/release/lib${LIB_NAME}.a" \
  "target/x86_64-apple-ios/release/lib${LIB_NAME}.a" \
  -output "$SIM_FAT"

cargo run --locked --release --features cli \
  -p auki-portable-echo-swift --bin uniffi-bindgen -- generate \
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
  -output "$ARTIFACTS/AukiPortableEcho.xcframework"

swiftc \
  -typecheck \
  -I "$BINDINGS" \
  -Xcc "-fmodule-map-file=$BINDINGS/module.modulemap" \
  "$SWIFT_OUT"/*.swift
plutil -lint "$ARTIFACTS/AukiPortableEcho.xcframework/Info.plist"

echo "XCFramework: $ARTIFACTS/AukiPortableEcho.xcframework"
echo "Swift package: $SWIFT_DIR/Package/Package.swift"
