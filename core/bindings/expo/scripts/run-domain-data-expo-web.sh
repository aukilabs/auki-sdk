#!/usr/bin/env bash
# Actual Expo Web module + Metro against the loopback Domain data fixture.
set -euo pipefail
domain_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
domain_expo="$domain_root/core/bindings/expo"
domain_example="$domain_expo/example"
domain_artifacts="$domain_root/output/playwright/domain-data-expo-web-$$"
domain_session="domain-data-expo-web-$$"
mkdir -p "$domain_artifacts"

cd "$domain_expo"
npm run build
npm run typecheck
node -e '(async()=>{for(const port of [18114,18115])await new Promise((resolve,reject)=>{const s=require("node:net").createServer();s.once("error",reject);s.listen(port,"127.0.0.1",()=>s.close(resolve));});})().catch(()=>process.exit(1));'

domain_fixture_pid=""
domain_metro_pid=""
domain_browser=""
pw() {
  local domain_output domain_status
  set +e
  domain_output="$(npx --yes --package @playwright/cli@0.1.19 playwright-cli --session "$domain_session" "$@" 2>&1)"
  domain_status=$?
  set -e
  printf '%s\n' "$domain_output"
  if [[ $domain_status -ne 0 || "$domain_output" == *"### Error"* ]]; then return 1; fi
}
cleanup() {
  if [[ -n "$domain_browser" ]]; then pw close >/dev/null 2>&1 || true; fi
  for domain_pid in "$domain_metro_pid" "$domain_fixture_pid"; do
    if [[ -n "$domain_pid" ]]; then kill "$domain_pid" 2>/dev/null || true; wait "$domain_pid" 2>/dev/null || true; fi
  done
}
trap cleanup EXIT INT TERM

node "$domain_root/test-support/domain-data-local-fixture.mjs" 18114 > "$domain_artifacts/fixture.log" 2>&1 &
domain_fixture_pid=$!
node -e '(async()=>{for(let i=0;i<100;i++){try{if((await fetch("http://127.0.0.1:18114/__phase",{signal:AbortSignal.timeout(1000)})).ok)return;}catch{}await new Promise(r=>setTimeout(r,100));}throw Error("fixture not ready");})().catch(e=>{console.error(e.message);process.exit(1);});'
kill -0 "$domain_fixture_pid"
cd "$domain_example"
EXPO_PUBLIC_AUKI_DOMAIN_DATA_TEST=1 CI=1 EXPO_NO_TELEMETRY=1 \
  node node_modules/expo/bin/cli start --offline --port 18115 > "$domain_artifacts/metro.log" 2>&1 &
domain_metro_pid=$!
node -e '(async()=>{for(let i=0;i<300;i++){try{if((await fetch("http://127.0.0.1:18115/status")).ok)return;}catch{}await new Promise(r=>setTimeout(r,100));}throw Error("Metro not ready");})().catch(e=>{console.error(e.message);process.exit(1);});'

cd "$domain_artifacts"
domain_browser=1
pw open http://127.0.0.1:18115/ --browser chrome
node -e '(async()=>{for(let i=0;i<900;i++){try{const p=await(await fetch("http://127.0.0.1:18114/__phase",{signal:AbortSignal.timeout(1000)})).json();if(p.phase==="failed")throw Error(JSON.stringify(p));if(p.phase==="passed"){if(p.count!==8)throw Error("incorrect case count");return;}}catch(e){if(String(e).includes("failed")||String(e).includes("case count"))throw e;}await new Promise(r=>setTimeout(r,100));}throw Error("Expo Web host did not finish");})().catch(e=>{console.error(e.message);process.exit(1);});'
pw snapshot | tee "$domain_artifacts/result.log"
rg -q 'PASS 8 Expo Domain data host cases' "$domain_artifacts/result.log"
printf 'Expo Web Domain data checks passed; artifacts: %s\n' "$domain_artifacts"
