#!/usr/bin/env bash
# Build the auki-identity-swift XCFramework + generated Swift bindings.
#
# Validated on rustc 1.94 + Xcode 26.3 against the three Apple targets
# below. Produces a two-slice AukiIdentity.xcframework (device ios-arm64
# + fat simulator ios-arm64_x86_64-simulator) plus the generated Swift
# glue (auki_identity_swift.swift) in $OUT/swift/, kept *outside* the
# xcframework Headers dir so SwiftPM consumers can pick it up at the
# package level while the xcframework Headers stay clean (FFI header +
# modulemap only).
#
# Same TLS-backend story as auki-network-swift PR #152: rustls `ring`
# 0.17 has first-class iOS cross-compile support so no CC/SDK env
# intervention is required. PR A doesn't pull swarm/libp2p so no
# SystemConfiguration.framework consumer-link concern applies here yet
# (it lands when PR B's network expansion brings the swarm feature in).
#
# Prereqs:
#   rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios
set -euo pipefail

CRATE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "$CRATE_DIR/../../.." && pwd)"
LIB_NAME="auki_identity_swift"
OUT="$CRATE_DIR/target-xcframework"
BINDINGS="$OUT/bindings"
mkdir -p "$BINDINGS"

cd "$WORKSPACE_ROOT"

# 1. Build the static lib for device + both simulator arches.
for TARGET in aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios; do
  cargo build --release -p auki-identity-swift --target "$TARGET"
done

# 2. Fat static lib for the simulator (xcframework rejects two slices
#    with the same platform); device lib stays standalone.
DEVICE_LIB="target/aarch64-apple-ios/release/lib${LIB_NAME}.a"
SIM_FAT="$OUT/lib${LIB_NAME}-sim.a"
lipo -create \
  "target/aarch64-apple-ios-sim/release/lib${LIB_NAME}.a" \
  "target/x86_64-apple-ios/release/lib${LIB_NAME}.a" \
  -output "$SIM_FAT"

# 3. Generate the Swift bindings + FFI header/modulemap from the device
#    static lib. Generating against the device .a is correct: uniffi-bindgen
#    reads arch-independent UNIFFI_META_* symbols via the `object` crate, so
#    the resulting .swift/.h/.modulemap are correct for both slices.
cargo run --release --features cli -p auki-identity-swift --bin uniffi-bindgen -- generate \
  --library "$DEVICE_LIB" \
  --language swift \
  --out-dir "$BINDINGS"
# Multi-namespace UniFFI output: with upstream crates each calling
# `setup_scaffolding!()` behind the `swift-bindings` feature, uniffi-bindgen
# emits one set of {.swift, *FFI.h, *FFI.modulemap} per namespace. The
# binding crate's `setup_scaffolding!()` produces the cdylib entry-point
# but its own namespace is mostly empty. Consolidate the modulemaps into
# a single `module.modulemap` declaring all FFI modules so Xcode/SwiftPM
# can find them as one umbrella module.
{
  for mm in "$BINDINGS"/*FFI.modulemap; do
    [ -f "$mm" ] || continue
    cat "$mm"
    echo
  done
} > "$BINDINGS/module.modulemap.tmp"
mv "$BINDINGS/module.modulemap.tmp" "$BINDINGS/module.modulemap"
# Remove the now-merged per-namespace modulemaps.
find "$BINDINGS" -name "*FFI.modulemap" -delete

# All Swift glue files are consumed at the SwiftPM-package level, not
# embedded in the xcframework — move every .swift out of $BINDINGS so
# step 4's `-headers $BINDINGS` packages only the FFI .h + modulemap.
SWIFT_OUT="$OUT/swift"
mkdir -p "$SWIFT_OUT"
for sf in "$BINDINGS"/*.swift; do
  [ -f "$sf" ] || continue
  mv "$sf" "$SWIFT_OUT/"
done

# 4. Assemble the XCFramework (headers = the FFI .h + modulemap only).
rm -rf "$OUT/AukiIdentity.xcframework"
xcodebuild -create-xcframework \
  -library "$DEVICE_LIB" -headers "$BINDINGS" \
  -library "$SIM_FAT"    -headers "$BINDINGS" \
  -output "$OUT/AukiIdentity.xcframework"

echo "XCFramework: $OUT/AukiIdentity.xcframework"
echo "Swift glue : $SWIFT_OUT/${LIB_NAME}.swift"
