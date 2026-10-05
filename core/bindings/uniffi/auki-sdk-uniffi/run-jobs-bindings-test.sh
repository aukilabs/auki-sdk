#!/usr/bin/env bash
# Exercise the typed Swift jobs layer against a fake generated UniFFI object.
set -euo pipefail

swift_crate="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$swift_crate/../../../.." && pwd)"
headers="$swift_crate/target-xcframework/bindings"

cd "$repo_root"
test -f "$headers/module.modulemap"
test -f "$swift_crate/Sources/AukiSDK/Generated/auki_sdk_uniffi.swift"

cargo build -p auki-sdk-uniffi --release --features standard-protocols --locked
mkdir -p target/jobs-swift-host
swiftc -parse-as-library -target "$(uname -m)-apple-macos$(sw_vers -productVersion)" \
  -I "$headers" -Xcc "-fmodule-map-file=$headers/module.modulemap" \
  "$swift_crate"/Sources/AukiSDK/Generated/*.swift \
  "$swift_crate"/Sources/AukiSDK/AukiSDK.swift "$swift_crate"/Sources/AukiSDK/Jobs.swift \
  "$swift_crate/tests/jobs-host.swift" \
  target/release/libauki_sdk_uniffi.a -framework SystemConfiguration -framework CoreFoundation -liconv \
  -o target/jobs-swift-host/host

target/jobs-swift-host/host
