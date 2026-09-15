#!/usr/bin/env bash
# Run the Release Expo/Hermes app against the loopback Domain data fixture.
set -euo pipefail
domain_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
domain_app="${AUKI_DOMAIN_DATA_IOS_APP:-$domain_root/target/domain-data-expo-ios-app/DerivedData/Build/Products/Release-iphonesimulator/ZitadelHandoffTest.app}"
test -f "$domain_app/main.jsbundle"
node -e 'const s=require("node:net").createServer();s.once("error",()=>process.exit(1));s.listen(18114,"127.0.0.1",()=>s.close());'
domain_artifacts="$domain_root/target/domain-data-expo-ios-app/run-$$"
mkdir -p "$domain_artifacts"
domain_device="${AUKI_DOMAIN_DATA_SIMULATOR_UDID:-}"
domain_created=""
domain_fixture_pid=""
if [[ -z "$domain_device" ]]; then
  domain_device="$(xcrun simctl create "Auki Domain data local $$" com.apple.CoreSimulator.SimDeviceType.iPhone-17 "${AUKI_DOMAIN_DATA_SIMULATOR_RUNTIME:-com.apple.CoreSimulator.SimRuntime.iOS-26-2}")"
  domain_created=1
fi
cleanup() {
  xcrun simctl io "$domain_device" screenshot "$domain_artifacts/final-screen.png" >/dev/null 2>&1 || true
  xcrun simctl terminate "$domain_device" com.auki.zitadeltodo.z09 >/dev/null 2>&1 || true
  if [[ -n "$domain_fixture_pid" ]]; then kill "$domain_fixture_pid" 2>/dev/null || true; wait "$domain_fixture_pid" 2>/dev/null || true; fi
  if [[ -n "$domain_created" ]]; then xcrun simctl delete "$domain_device" >/dev/null 2>&1 || true; fi
}
trap cleanup EXIT INT TERM

if [[ -n "$domain_created" ]]; then xcrun simctl boot "$domain_device"; fi
xcrun simctl bootstatus "$domain_device" -b
node "$domain_root/test-support/domain-data-local-fixture.mjs" 18114 > "$domain_artifacts/fixture.log" 2>&1 &
domain_fixture_pid=$!
node -e '(async()=>{for(let i=0;i<100;i++){try{if((await fetch("http://127.0.0.1:18114/__phase",{signal:AbortSignal.timeout(1000)})).ok)return;}catch{}await new Promise(r=>setTimeout(r,100));}throw Error("fixture not ready");})().catch(e=>{console.error(e.message);process.exit(1);});'
kill -0 "$domain_fixture_pid"
xcrun simctl install "$domain_device" "$domain_app"
xcrun simctl launch "$domain_device" com.auki.zitadeltodo.z09
node -e '(async()=>{for(let i=0;i<900;i++){try{const p=await(await fetch("http://127.0.0.1:18114/__phase",{signal:AbortSignal.timeout(1000)})).json();if(p.phase==="failed")throw Error(JSON.stringify(p));if(p.phase==="passed"){if(p.count!==8)throw Error("incorrect case count");console.log(JSON.stringify(p));return;}}catch(e){if(String(e).includes("failed")||String(e).includes("case count"))throw e;}await new Promise(r=>setTimeout(r,100));}throw Error("Expo iOS host did not finish");})().catch(e=>{console.error(e.message);process.exit(1);});' | tee "$domain_artifacts/result.log"
xcrun simctl io "$domain_device" screenshot "$domain_artifacts/result.png"
printf 'Expo iOS Domain data checks passed; device %s; artifacts: %s\n' "$domain_device" "$domain_artifacts"
