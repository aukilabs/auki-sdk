#!/usr/bin/env bash
# Build the auki-network-swift XCFramework + generated Swift bindings.
#
# Validated 2026-05-19 against rustc 1.94 + Xcode 26.3 on the three
# Apple targets below. Produces a two-slice AukiNetwork.xcframework
# (device ios-arm64 + fat simulator ios-arm64_x86_64-simulator) plus
# the generated Swift glue (auki_network_swift.swift) in $OUT/swift/,
# kept *outside* the xcframework Headers dir so SwiftPM consumers can
# pick it up at the package level while the xcframework Headers stay
# clean (FFI header + modulemap only). Active TLS backend on iOS is
# `ring 0.17` via reqwest's `rustls-tls` default (no aws-lc-rs);
# ring 0.17.x has first-class iOS cross-compile support so no CC/SDK
# env intervention is required.
#
# Heads-up for Stage 2: when the `swarm` feature gets pulled (libp2p),
# the consuming Xcode target may need to link `SystemConfiguration.
# framework` if if-watch symbols surface; not the case at Stage 1.
#
# Prereqs:
#   rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios
set -euo pipefail

CRATE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "$CRATE_DIR/../.." && pwd)"
LIB_NAME="auki_network_swift"
OUT="$CRATE_DIR/target-xcframework"
BINDINGS="$OUT/bindings"
mkdir -p "$BINDINGS"

cd "$WORKSPACE_ROOT"

# 1. Build the static lib for device + both simulator arches.
for TARGET in aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios; do
  cargo build --release -p auki-network-swift --target "$TARGET"
done

# 2. Fat static lib for the simulator (xcframework rejects two slices
#    with the same platform), device lib stays standalone.
DEVICE_LIB="target/aarch64-apple-ios/release/lib${LIB_NAME}.a"
SIM_FAT="$OUT/lib${LIB_NAME}-sim.a"
lipo -create \
  "target/aarch64-apple-ios-sim/release/lib${LIB_NAME}.a" \
  "target/x86_64-apple-ios/release/lib${LIB_NAME}.a" \
  -output "$SIM_FAT"

# 3. Generate the Swift bindings + FFI header/modulemap from the built
#    library (UniFFI 0.31 library mode auto-detects; no UDL).
#    Generating against the *device* .a is fine: uniffi-bindgen reads
#    arch-independent UNIFFI_META_* symbols via the `object` crate, so
#    the resulting .swift/.h/.modulemap are correct for both slices.
cargo run --release --features cli --bin uniffi-bindgen -- generate \
  --library "$DEVICE_LIB" \
  --language swift \
  --out-dir "$BINDINGS"
# Xcode expects `module.modulemap`.
if [ -f "$BINDINGS/${LIB_NAME}FFI.modulemap" ]; then
  mv "$BINDINGS/${LIB_NAME}FFI.modulemap" "$BINDINGS/module.modulemap"
fi
# The Swift glue file is consumed at the SwiftPM-package level, not
# embedded in the xcframework — move it out of $BINDINGS so step 4's
# `-headers $BINDINGS` packages only the FFI header + modulemap.
SWIFT_OUT="$OUT/swift"
mkdir -p "$SWIFT_OUT"
mv "$BINDINGS/${LIB_NAME}.swift" "$SWIFT_OUT/${LIB_NAME}.swift"

# 4. Assemble the XCFramework (headers = the FFI .h + modulemap only).
rm -rf "$OUT/AukiNetwork.xcframework"
xcodebuild -create-xcframework \
  -library "$DEVICE_LIB" -headers "$BINDINGS" \
  -library "$SIM_FAT"    -headers "$BINDINGS" \
  -output "$OUT/AukiNetwork.xcframework"

echo "XCFramework: $OUT/AukiNetwork.xcframework"
echo "Swift glue : $SWIFT_OUT/${LIB_NAME}.swift"
