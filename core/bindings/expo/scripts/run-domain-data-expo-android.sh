#!/usr/bin/env bash
# Run the debug Expo Android app against the loopback Domain data fixture.
set -euo pipefail
domain_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
domain_example="$domain_root/core/bindings/expo/example"
domain_apk="${AUKI_DOMAIN_DATA_ANDROID_APK:-$domain_root/target/domain-data-expo-android-app/app-debug.apk}"
test -f "$domain_apk"
export ANDROID_HOME="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/Library/Android/sdk}}"
export PATH="$ANDROID_HOME/platform-tools:$ANDROID_HOME/emulator:$PATH"

node -e 'const s=require("node:net").createServer();s.once("error",()=>process.exit(1));s.listen(18114,"127.0.0.1",()=>s.close());'
node -e 'const s=require("node:net").createServer();s.once("error",()=>process.exit(1));s.listen(18115,"127.0.0.1",()=>s.close());'
domain_artifacts="$domain_root/target/domain-data-expo-android-app/run-$$"
mkdir -p "$domain_artifacts"
domain_emulator_pid=""
domain_fixture_pid=""
domain_metro_pid=""
domain_serial="${AUKI_ANDROID_SERIAL:-}"

cleanup() {
  if [[ -n "$domain_serial" ]]; then
    adb -s "$domain_serial" exec-out screencap -p > "$domain_artifacts/final-screen.png" 2>/dev/null || true
    adb -s "$domain_serial" shell am force-stop com.auki.zitadeltodo.z09 >/dev/null 2>&1 || true
  fi
  if [[ -n "$domain_fixture_pid" ]]; then kill "$domain_fixture_pid" 2>/dev/null || true; wait "$domain_fixture_pid" 2>/dev/null || true; fi
  if [[ -n "$domain_metro_pid" ]]; then kill "$domain_metro_pid" 2>/dev/null || true; wait "$domain_metro_pid" 2>/dev/null || true; fi
  if [[ -n "$domain_emulator_pid" ]]; then kill "$domain_emulator_pid" 2>/dev/null || true; wait "$domain_emulator_pid" 2>/dev/null || true; fi
}
trap cleanup EXIT INT TERM

if [[ -z "$domain_serial" ]]; then
  domain_serial="$(adb devices | awk 'NR>1 && $2=="device" { print $1; exit }')"
fi
if [[ -z "$domain_serial" ]]; then
  domain_avd="${AUKI_ANDROID_AVD:-$(emulator -list-avds | head -1)}"
  if [[ -z "$domain_avd" ]]; then
    echo "No running Android device and no AVD. Set AUKI_ANDROID_SERIAL or create an AVD." >&2
    exit 1
  fi
  emulator -avd "$domain_avd" -no-snapshot-save -netdelay none -netspeed full > "$domain_artifacts/emulator.log" 2>&1 &
  domain_emulator_pid=$!
  adb wait-for-device
  domain_serial="$(adb devices | awk 'NR>1 && $2=="device" { print $1; exit }')"
fi
adb -s "$domain_serial" wait-for-device
until [[ "$(adb -s "$domain_serial" shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')" == "1" ]]; do
  sleep 2
done

# Debug APKs load JavaScript from Metro. The public env flag selects the Domain data cases.
cd "$domain_example"
EXPO_PUBLIC_AUKI_DOMAIN_DATA_TEST=1 CI=1 EXPO_NO_TELEMETRY=1 \
  npx expo start --port 8081 --non-interactive \
  > "$domain_artifacts/metro.log" 2>&1 &
domain_metro_pid=$!
node -e '(async()=>{for(let i=0;i<150;i++){try{const t=await(await fetch("http://127.0.0.1:8081/status",{signal:AbortSignal.timeout(1000)})).text();if(t.includes("running"))return;}catch{}await new Promise(r=>setTimeout(r,200));}throw Error("Metro did not start");})().catch(e=>{console.error(e.message);process.exit(1);});'
adb -s "$domain_serial" reverse tcp:8081 tcp:8081
adb -s "$domain_serial" reverse tcp:18114 tcp:18114
adb -s "$domain_serial" reverse tcp:18115 tcp:18115
adb -s "$domain_serial" reverse tcp:18111 tcp:18111
curl -fsS -o "$domain_artifacts/index.bundle" \
  "http://127.0.0.1:8081/index.bundle?platform=android&dev=true&minify=false"
grep -q "Expo Domain data tests" "$domain_artifacts/index.bundle"

AUKI_DOMAIN_DATA_SERVER_PORT=18115 \
  node "$domain_root/test-support/domain-data-local-fixture.mjs" 18114 > "$domain_artifacts/fixture.log" 2>&1 &
domain_fixture_pid=$!
node -e '(async()=>{for(let i=0;i<100;i++){try{if((await fetch("http://127.0.0.1:18114/__phase",{signal:AbortSignal.timeout(1000)})).ok)return;}catch{}await new Promise(r=>setTimeout(r,100));}throw Error("fixture not ready");})().catch(e=>{console.error(e.message);process.exit(1);});'
kill -0 "$domain_fixture_pid"
adb -s "$domain_serial" install -r "$domain_apk" > "$domain_artifacts/install.log"
adb -s "$domain_serial" shell am start -n com.auki.zitadeltodo.z09/.MainActivity
node -e '(async()=>{for(let i=0;i<900;i++){try{const p=await(await fetch("http://127.0.0.1:18114/__phase",{signal:AbortSignal.timeout(1000)})).json();if(p.phase==="failed")throw Error(JSON.stringify(p));if(p.phase==="passed"){if(p.count!==8)throw Error("incorrect case count");console.log(JSON.stringify(p));return;}}catch(e){if(String(e).includes("failed")||String(e).includes("case count"))throw e;}await new Promise(r=>setTimeout(r,100));}throw Error("Expo Android host did not finish");})().catch(e=>{console.error(e.message);process.exit(1);});' | tee "$domain_artifacts/result.log"
adb -s "$domain_serial" exec-out screencap -p > "$domain_artifacts/result.png" || true
printf 'Expo Android Domain data checks passed; device %s; artifacts: %s\n' "$domain_serial" "$domain_artifacts"
