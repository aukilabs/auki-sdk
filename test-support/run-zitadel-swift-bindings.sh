#!/usr/bin/env bash
# Run a real Swift async host against the generated UniFFI API on macOS.
# Build the XCFramework first; this reuses its exact generated headers/Swift.
set -euo pipefail
z08_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$z08_root"
z08_crate="$z08_root/core/bindings/swift/auki-sdk-swift"
z08_headers="$z08_crate/target-xcframework/bindings"
test -f "$z08_headers/module.modulemap"
test -f "$z08_crate/Sources/AukiSDK/Generated/auki_sdk_swift.swift"
cargo build -p auki-sdk-swift --release --features standard-protocols --locked
mkdir -p target/zitadel-swift-host
swiftc -parse-as-library -target "$(uname -m)-apple-macos$(sw_vers -productVersion)" \
  -I "$z08_headers" -Xcc "-fmodule-map-file=$z08_headers/module.modulemap" \
  "$z08_crate"/Sources/AukiSDK/Generated/*.swift test-support/zitadel-swift-host.swift \
  target/release/libauki_sdk_swift.a -framework SystemConfiguration -framework CoreFoundation -liconv \
  -o target/zitadel-swift-host/host
node -e 'const s=require("node:net").createServer();s.once("error",()=>process.exit(1));s.listen(18111,"127.0.0.1",()=>s.close());'
z08_fixture_pid=""
cleanup() {
  if [[ -n "$z08_fixture_pid" ]]; then kill "$z08_fixture_pid" 2>/dev/null || true; wait "$z08_fixture_pid" 2>/dev/null || true; fi
}
trap cleanup EXIT INT TERM
node test-support/zitadel-bindings-fixture.mjs > target/zitadel-swift-host/fixture.log 2>&1 &
z08_fixture_pid=$!
node -e '(async()=>{for(let i=0;i<100;i++){try{if((await fetch("http://127.0.0.1:18111/__stats")).ok)return;}catch{}await new Promise(r=>setTimeout(r,100));}throw Error("fixture not ready");})().catch(()=>process.exit(1));'
kill -0 "$z08_fixture_pid"
target/zitadel-swift-host/host | tee target/zitadel-swift-host/result.log
rg -q '^PASS 8 Swift binding cases$' target/zitadel-swift-host/result.log
