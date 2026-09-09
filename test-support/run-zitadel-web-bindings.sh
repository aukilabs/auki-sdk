#!/usr/bin/env bash
set -euo pipefail
: "${WASM_BINDGEN_TEST_RUNNER:?Set to wasm-bindgen-test-runner 0.2.121, matching Cargo.lock}"
test -x "$WASM_BINDGEN_TEST_RUNNER"
z08_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$z08_root/core/bindings/web/auki-sdk-web"
npm run check
node -e '(async()=>{for(const port of [18111,18112])await new Promise((resolve,reject)=>{const s=require("node:net").createServer();s.once("error",reject);s.listen(port,"127.0.0.1",()=>s.close(resolve));});})().catch(()=>process.exit(1));'
cd "$z08_root"
z08_wasm="$(cargo test -p auki-sdk-web --lib --features finite-protocols,message,stream --target wasm32-unknown-unknown --no-run --locked --message-format=json | node -e 'let data="";process.stdin.on("data",c=>data+=c);process.stdin.on("end",()=>{const files=data.split("\n").filter(Boolean).map(s=>JSON.parse(s)).filter(v=>v.reason==="compiler-artifact"&&v.target.name==="auki_sdk_web"&&v.profile.test).flatMap(v=>v.filenames).filter(f=>f.endsWith(".wasm"));if(files.length!==1)process.exit(1);process.stdout.write(files[0]);});')"
z08_artifacts="$z08_root/output/playwright/zitadel-z08-$$"
mkdir -p "$z08_artifacts"
cd "$z08_artifacts"
z08_session="zitadel-z08-$$"
z08_fixture_pid=""
z08_browser=""
z08_runner_pid=""
pw() {
  local z08_output
  z08_output="$(npx --yes --package @playwright/cli@0.1.19 playwright-cli --session "$z08_session" "$@")" || return $?
  printf '%s\n' "$z08_output"
  if rg -q '^### Error' <<< "$z08_output"; then return 1; fi
}
cleanup() {
  if [[ -n "$z08_browser" ]]; then pw close >/dev/null 2>&1 || true; fi
  if [[ -n "$z08_fixture_pid" ]]; then kill "$z08_fixture_pid" 2>/dev/null || true; wait "$z08_fixture_pid" 2>/dev/null || true; fi
  if [[ -n "$z08_runner_pid" ]]; then kill "$z08_runner_pid" 2>/dev/null || true; wait "$z08_runner_pid" 2>/dev/null || true; fi
}
trap cleanup EXIT INT TERM
node "$z08_root/test-support/zitadel-bindings-fixture.mjs" > fixture.log 2>&1 &
z08_fixture_pid=$!
node -e '(async()=>{for(let i=0;i<100;i++){try{if((await fetch("http://127.0.0.1:18111/__stats")).ok)return;}catch{}await new Promise(r=>setTimeout(r,100));}throw Error("fixture not ready");})().catch(()=>process.exit(1));'
kill -0 "$z08_fixture_pid"
z08_browser=1
pw open http://127.0.0.1:18111/ --browser chrome
pw snapshot
pw run-code 'async (page) => { await page.waitForFunction(() => /^(PASS|FAIL)/.test(document.querySelector("#result").textContent), null, {timeout:45000}); const text = await page.locator("#result").innerText(); if (!text.startsWith("PASS 5 Web binding cases")) throw new Error(text); console.log(text); }'
pw snapshot
NO_HEADLESS=1 WASM_BINDGEN_TEST_ADDRESS=127.0.0.1:18112 "$WASM_BINDGEN_TEST_RUNNER" "$z08_wasm" > wasm-runner.log 2>&1 &
z08_runner_pid=$!
node -e '(async()=>{for(let i=0;i<200;i++){try{if((await fetch("http://127.0.0.1:18112/")).ok)return;}catch{}await new Promise(r=>setTimeout(r,100));}throw Error("runner not ready");})().catch(()=>process.exit(1));'
kill -0 "$z08_runner_pid"
pw goto http://127.0.0.1:18112/
pw snapshot
pw run-code 'async (page) => { await page.waitForFunction(() => document.body.textContent.includes("test result:"), null, {timeout:45000}); const result = await page.locator("body").innerText(); const match = result.match(/test result: ok\. (\d+) passed; 0 failed; 0 ignored;/); if (!match || Number(match[1]) < 41) throw new Error(result); console.log(result); }'
pw snapshot
printf 'Z08 generated Web binding checks passed; artifacts: %s\n' "$z08_artifacts"
