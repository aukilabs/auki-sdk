#!/usr/bin/env bash
# Offline Chromium proof for Web session/data/jobs and cancellation behavior.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
: "${WASM_BINDGEN_TEST_RUNNER:?Set this to wasm-bindgen-test-runner matching Cargo.lock (0.2.121)}"
test -x "$WASM_BINDGEN_TEST_RUNNER"
for tool in cargo node npx; do command -v "$tool" >/dev/null; done
node -e 'const s=require("node:net").createServer();s.once("error",()=>process.exit(1));s.listen(18138,"127.0.0.1",()=>s.close());'
wasm="$(cargo test --locked -p auki-sdk-web --lib --target wasm32-unknown-unknown --no-run --message-format=json | node -e 'let s="";process.stdin.on("data",c=>s+=c);process.stdin.on("end",()=>{const files=s.split("\n").filter(Boolean).map(JSON.parse).filter(v=>v.reason==="compiler-artifact"&&v.target.name==="auki_sdk_web"&&v.profile.test).flatMap(v=>v.filenames).filter(f=>f.endsWith(".wasm"));if(files.length!==1)process.exit(1);process.stdout.write(files[0]);});')"
artifacts="$root/output/playwright/domain-data-$$"
mkdir -p "$artifacts"
cd "$artifacts"
session="domain-data-$$"
runner_pid=""
browser_started=""
pw() {
    local output
    output="$(npx --yes --package @playwright/cli@0.1.19 playwright-cli --session "$session" "$@")" || return $?
    printf '%s\n' "$output"
    if rg -q '^### Error' <<< "$output"; then return 1; fi
}
cleanup() {
    if [[ -n "$browser_started" ]]; then pw close >/dev/null 2>&1 || true; fi
    if [[ -n "$runner_pid" ]]; then kill "$runner_pid" 2>/dev/null || true; wait "$runner_pid" 2>/dev/null || true; fi
}
trap cleanup EXIT INT TERM
NO_HEADLESS=1 WASM_BINDGEN_TEST_ADDRESS=127.0.0.1:18138 "$WASM_BINDGEN_TEST_RUNNER" "$wasm" > runner.log 2>&1 &
runner_pid=$!
node -e '(async()=>{for(let i=0;i<300;i++){try{if((await fetch("http://127.0.0.1:18138/",{signal:AbortSignal.timeout(1000)})).ok)return;}catch{}await new Promise(r=>setTimeout(r,100));}throw Error("runner did not become ready");})().catch(e=>{console.error(e.message);process.exit(1);});'
browser_started=1
pw open http://127.0.0.1:18138/ --browser chrome
pw snapshot
pw run-code 'async page => { await page.waitForFunction(() => document.body.textContent.includes("test result:"), null, {timeout:45000}); const result = await page.locator("body").innerText(); if (!result.includes("test result: ok. 10 passed; 0 failed; 0 ignored;")) throw Error(result); return result; }'
pw snapshot
printf 'Web data browser tests passed; artifacts: %s\n' "$artifacts"
