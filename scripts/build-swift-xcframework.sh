#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: $0 <crate>" >&2
  exit 2
fi

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "build-swift-xcframework requires macOS because it uses xcodebuild and lipo" >&2
  exit 1
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

crate="$1"
lib_name="${crate//-/_}"
package_dir="bindings/swift/${crate}"
generated_dir="${package_dir}/generated"
headers_dir="${generated_dir}/headers"

rm -f "$package_dir/${lib_name}.swift" "$package_dir/${lib_name}FFI.h" "$package_dir/${lib_name}FFI.modulemap"
rm -rf "$generated_dir"
mkdir -p "$headers_dir"

for target in aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios aarch64-apple-darwin x86_64-apple-darwin; do
  cargo build --release -p "$crate" --target "$target"
done

device_lib="target/aarch64-apple-ios/release/lib${lib_name}.a"
sim_fat="${generated_dir}/lib${lib_name}-sim.a"
macos_fat="${generated_dir}/lib${lib_name}-macos.a"

lipo -create \
  "target/aarch64-apple-ios-sim/release/lib${lib_name}.a" \
  "target/x86_64-apple-ios/release/lib${lib_name}.a" \
  -output "$sim_fat"

lipo -create \
  "target/aarch64-apple-darwin/release/lib${lib_name}.a" \
  "target/x86_64-apple-darwin/release/lib${lib_name}.a" \
  -output "$macos_fat"

cargo run -p "$crate" --release --features cli --bin uniffi-bindgen -- generate \
  --library "$device_lib" \
  --language swift \
  --out-dir "$headers_dir"

mv "$headers_dir/${lib_name}.swift" "$generated_dir/${lib_name}.swift"
if [[ -f "$headers_dir/${lib_name}FFI.modulemap" ]]; then
  mv "$headers_dir/${lib_name}FFI.modulemap" "$headers_dir/module.modulemap"
fi

xcodebuild -create-xcframework \
  -library "$device_lib" -headers "$headers_dir" \
  -library "$sim_fat" -headers "$headers_dir" \
  -library "$macos_fat" -headers "$headers_dir" \
  -output "$generated_dir/${lib_name}.xcframework"

rm -f "$sim_fat" "$macos_fat"

echo "Generated Swift package with XCFramework in $generated_dir"
