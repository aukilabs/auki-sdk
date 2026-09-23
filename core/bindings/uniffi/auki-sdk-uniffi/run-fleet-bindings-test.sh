#!/usr/bin/env bash
# Exercise the generated UniFFI client and typed Swift API against local fixtures.
set -euo pipefail
swift_crate="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$swift_crate/../../../.." && pwd)"
headers="$swift_crate/target-xcframework/bindings"
cd "$repo_root"
test -f "$headers/module.modulemap"
cargo build -p auki-sdk-swift --release --features standard-protocols --locked
mkdir -p target/fleet-swift-host
swiftc -parse-as-library -target "$(uname -m)-apple-macos$(sw_vers -productVersion)" \
  -I "$headers" -Xcc "-fmodule-map-file=$headers/module.modulemap" \
  "$swift_crate"/Sources/AukiSDK/Generated/*.swift \
  "$swift_crate"/Sources/AukiSDK/*.swift "$swift_crate/tests/fleet-host.swift" \
  target/release/libauki_sdk_swift.a -framework SystemConfiguration -framework CoreFoundation -liconv \
  -o target/fleet-swift-host/host
python3 test-support/fleet_fixture.py target/fleet-swift-host/host
