#!/usr/bin/env bash
# Run the Release Expo/Hermes application in an isolated iOS simulator.
# Build/setup commands are documented alongside the example.
set -euo pipefail
z09_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$z09_root"
z09_app="$z09_root/target/zitadel-z09-ios/DerivedData/Build/Products/Release-iphonesimulator/ZitadelHandoffTest.app"
test -f "$z09_app/main.jsbundle"
node -e 'const s=require("node:net").createServer();s.once("error",()=>process.exit(1));s.listen(18111,"127.0.0.1",()=>s.close());'
z09_artifacts="$z09_root/target/zitadel-z09-ios/run-$$"
mkdir -p "$z09_artifacts"
z09_created=""
z09_fixture_pid=""
z09_device="${ZITADEL_SIMULATOR_UDID:-}"
if [[ -z "$z09_device" ]]; then
  z09_device="$(xcrun simctl create "Zitadel Z09 local acceptance $$" com.apple.CoreSimulator.SimDeviceType.iPhone-17 "${ZITADEL_SIMULATOR_RUNTIME:-com.apple.CoreSimulator.SimRuntime.iOS-26-2}")"
  z09_created=1
fi
cleanup() {
  xcrun simctl io "$z09_device" screenshot "$z09_artifacts/final-screen.png" >/dev/null 2>&1 || true
  xcrun simctl terminate "$z09_device" com.auki.zitadeltodo.z09 >/dev/null 2>&1 || true
  if [[ -n "$z09_fixture_pid" ]]; then kill "$z09_fixture_pid" 2>/dev/null || true; wait "$z09_fixture_pid" 2>/dev/null || true; fi
  if [[ -n "$z09_created" ]]; then xcrun simctl shutdown "$z09_device" >/dev/null 2>&1 || true; fi
}
xcrun simctl list devices --json | node -e 'let data="";process.stdin.on("data",c=>data+=c);process.stdin.on("end",()=>{const d=Object.values(JSON.parse(data).devices).flat().find(d=>d.udid===process.argv[1]);if(!d||!d.name.startsWith("Zitadel Z09"))process.exit(1);});' "$z09_device"
trap cleanup EXIT INT TERM
if [[ -n "$z09_created" ]]; then xcrun simctl boot "$z09_device"; fi
xcrun simctl bootstatus "$z09_device" -b
node test-support/zitadel-bindings-fixture.mjs > "$z09_artifacts/fixture.log" 2>&1 &
z09_fixture_pid=$!
xcrun simctl install "$z09_device" "$z09_app"
xcrun simctl launch "$z09_device" com.auki.zitadeltodo.z09
wait_phase() {
  node -e 'const wanted=process.argv[1];(async()=>{const start=Date.now();while(Date.now()-start<90000){try{const p=await(await fetch("http://127.0.0.1:18111/__phase",{signal:AbortSignal.timeout(1000)})).json();if(p.phase==="failed")throw new Error(JSON.stringify(p));if(p.phase===wanted){if(wanted==="passed"&&p.count!==7)throw Error("incorrect test count");console.log(JSON.stringify(p));return;}}catch(e){if(String(e).includes("failed")||String(e).includes("test count"))throw e;}await new Promise(r=>setTimeout(r,100));}throw Error("Expo host did not reach "+wanted);})().catch(e=>{console.error(e.message);process.exit(1);});' "$1"
}
wait_phase suspend-ready
xcrun simctl launch "$z09_device" com.apple.mobilesafari
node -e 'setTimeout(()=>{},5000)'
xcrun simctl launch "$z09_device" com.auki.zitadeltodo.z09
node -e 'fetch("http://127.0.0.1:18111/__phase",{method:"POST",body:JSON.stringify({phase:"resumed"})}).then(r=>{if(!r.ok)process.exit(1);}).catch(()=>process.exit(1));'
wait_phase passed | tee "$z09_artifacts/result.log"
xcrun simctl io "$z09_device" screenshot "$z09_artifacts/result.png"
printf 'Z09 actual Expo iOS checks passed; device %s; artifacts: %s\n' "$z09_device" "$z09_artifacts"
