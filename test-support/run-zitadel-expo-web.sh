#!/usr/bin/env bash
# Actual Expo Web module + Metro, not a mocked Expo facade.
set -euo pipefail
z09_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
z09_example="$z09_root/bindings/expo/example"
cd "$z09_root/bindings/expo"
npm run build
npm run typecheck
node -e '(async()=>{for(const port of [18111,18113])await new Promise((resolve,reject)=>{const s=require("node:net").createServer();s.once("error",reject);s.listen(port,"127.0.0.1",()=>s.close(resolve));});})().catch(()=>process.exit(1));'
z09_artifacts="$z09_root/output/playwright/zitadel-z09-web-$$"
mkdir -p "$z09_artifacts"
z09_session="zitadel-z09-web-$$"
z09_fixture_pid=""
z09_metro_pid=""
z09_browser=""
pw() {
  local z09_output z09_status=0
  z09_output="$(npx --yes --package @playwright/cli@0.1.19 playwright-cli --session "$z09_session" "$@")" || z09_status=$?
  printf '%s\n' "$z09_output"
  if [[ "$z09_status" != 0 ]]; then return "$z09_status"; fi
  if rg -q '^### Error' <<< "$z09_output"; then return 1; fi
}
cleanup() {
  if [[ -n "$z09_browser" ]]; then pw close >/dev/null 2>&1 || true; fi
  for z09_pid in "$z09_metro_pid" "$z09_fixture_pid"; do
    if [[ -n "$z09_pid" ]]; then kill "$z09_pid" 2>/dev/null || true; wait "$z09_pid" 2>/dev/null || true; fi
  done
}
trap cleanup EXIT INT TERM
node "$z09_root/test-support/zitadel-bindings-fixture.mjs" > "$z09_artifacts/fixture.log" 2>&1 &
z09_fixture_pid=$!
cd "$z09_example"
CI=1 EXPO_NO_TELEMETRY=1 node node_modules/expo/bin/cli start --offline --port 18113 > "$z09_artifacts/metro.log" 2>&1 &
z09_metro_pid=$!
node -e '(async()=>{for(let i=0;i<300;i++){try{if((await fetch("http://127.0.0.1:18113/status")).ok)return;}catch{}await new Promise(r=>setTimeout(r,100));}throw Error("Metro not ready");})().catch(()=>process.exit(1));'
kill -0 "$z09_metro_pid"
kill -0 "$z09_fixture_pid"
cd "$z09_artifacts"
z09_browser=1
pw open http://127.0.0.1:18113/ --browser chrome
pw snapshot
pw run-code 'async (page) => { const start=Date.now(); let phase; while(Date.now()-start<60000){phase=await(await page.request.get("http://127.0.0.1:18111/__phase")).json(); if(phase.phase==="failed")throw new Error(JSON.stringify(phase)); if(phase.phase==="suspend-ready")break; await page.waitForTimeout(100);} if(phase.phase!=="suspend-ready")throw new Error(await page.locator("body").innerText()); const cdp=await page.context().newCDPSession(page); await cdp.send("Page.setWebLifecycleState",{state:"frozen"}); try{await page.waitForTimeout(5000);}finally{await cdp.send("Page.setWebLifecycleState",{state:"active"});} await page.request.post("http://127.0.0.1:18111/__phase",{data:{phase:"resumed"}}); await page.waitForFunction(()=>document.body.textContent.includes("PASS 7 Expo host cases")||document.body.textContent.includes("FAIL"),null,{timeout:30000}); const text=await page.locator("body").innerText(); if(!text.includes("PASS 7 Expo host cases"))throw new Error(text); console.log(text); }'
pw snapshot
printf 'Z09 Expo Web checks passed; artifacts: %s\n' "$z09_artifacts"
