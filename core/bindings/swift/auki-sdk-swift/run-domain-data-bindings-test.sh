#!/usr/bin/env bash
# Exercise the generated Swift UniFFI API against the shared loopback fixture.
set -euo pipefail

swift_crate="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$swift_crate/../../../.." && pwd)"
headers="$swift_crate/target-xcframework/bindings"
port="${DOMAIN_DATA_SWIFT_FIXTURE_PORT:-18141}"
base_url="http://127.0.0.1:$port"

cd "$repo_root"
test -f "$headers/module.modulemap"
test -f "$swift_crate/Sources/AukiSDK/Generated/auki_sdk_swift.swift"

cargo build -p auki-sdk-swift --release --features standard-protocols --locked
mkdir -p target/domain-data-swift-host
swiftc -parse-as-library -target "$(uname -m)-apple-macos$(sw_vers -productVersion)" \
  -I "$headers" -Xcc "-fmodule-map-file=$headers/module.modulemap" \
  "$swift_crate"/Sources/AukiSDK/Generated/*.swift "$swift_crate/tests/domain-data-host.swift" \
  target/release/libauki_sdk_swift.a -framework SystemConfiguration -framework CoreFoundation -liconv \
  -o target/domain-data-swift-host/host

node -e 'const s=require("node:net").createServer();s.once("error",()=>process.exit(1));s.listen(Number(process.argv[1]),"127.0.0.1",()=>s.close());' "$port"
fixture_pid=""
cleanup() {
  if [[ -n "$fixture_pid" ]]; then
    kill "$fixture_pid" 2>/dev/null || true
    wait "$fixture_pid" 2>/dev/null || true
  fi
}
trap cleanup EXIT INT TERM

node test-support/domain-data-local-fixture.mjs "$port" > target/domain-data-swift-host/fixture.log 2>&1 &
fixture_pid=$!
node -e '(async()=>{for(let i=0;i<100;i++){try{if((await fetch(process.argv[1]+"/__stats")).ok)return;}catch{}await new Promise(r=>setTimeout(r,100));}throw Error("fixture not ready");})().catch(()=>process.exit(1));' "$base_url"

target/domain-data-swift-host/host "$base_url"
node -e 'fetch(process.argv[1]+"/__stats").then(r=>r.json()).then(s=>{if(s.refreshes!==1||s.importedRefreshes!==1||s.exchanges!==7||s.p2pExchanges!==1||s.multipartCompletions!==1||s.multipartAborts!==1||s.outstandingUploads!==0||s.records!==1||s.requests["GET /api/v1/domains"]!==3||s.requests["GET /api/v1/accessible-domains"]!==1||s.requests["GET /api/v1/domain-discovery/zitadel"]!==1||s.requests["POST /api/v1/domains/aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa/auth/zitadel"]!==1)throw Error(JSON.stringify(s));console.log("PASS Swift FFI renewal, owner/viewer imported listing, data, persistence, and cleanup counters")}).catch(e=>{console.error(e);process.exit(1)})' "$base_url"
