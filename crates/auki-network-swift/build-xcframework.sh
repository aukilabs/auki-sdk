#!/usr/bin/env bash
# Build the auki-network-swift XCFramework + generated Swift bindings.
#
# STATUS: scaffolding. The host `cargo build/test -p auki-network-swift`
# is the validated Stage 1 gate; this script encodes the standard
# UniFFI 0.31 iOS flow but has NOT been run/verified here. Known sharp
# edges to expect on first real run (see parking_lot.md):
#   - `ring` cross-compile to aarch64-apple-ios may need CC/SDK env;
#     `aws-lc-rs` is the iOS-friendly fallback.
#   - link `SystemConfiguration.framework` in the consuming Xcode target
#     if libp2p/if-watch symbols surface (Stage 2+, swarm feature).
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
cargo run --release --features cli --bin uniffi-bindgen -- generate \
  --library "$DEVICE_LIB" \
  --language swift \
  --out-dir "$BINDINGS"
# Xcode expects `module.modulemap`.
if [ -f "$BINDINGS/${LIB_NAME}FFI.modulemap" ]; then
  mv "$BINDINGS/${LIB_NAME}FFI.modulemap" "$BINDINGS/module.modulemap"
fi

# 4. Assemble the XCFramework (headers = the generated FFI dir).
rm -rf "$OUT/AukiNetwork.xcframework"
xcodebuild -create-xcframework \
  -library "$DEVICE_LIB" -headers "$BINDINGS" \
  -library "$SIM_FAT"    -headers "$BINDINGS" \
  -output "$OUT/AukiNetwork.xcframework"

echo "XCFramework: $OUT/AukiNetwork.xcframework"
echo "Swift glue : $BINDINGS/${LIB_NAME}.swift"
