#!/usr/bin/env bash
# Real Chrome + Wasm runtime proof; task-owned processes and synthetic credentials.
set -euo pipefail

z07_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$z07_root"
for z07_tool in cargo node npx; do command -v "$z07_tool" >/dev/null; done
: "${WASM_BINDGEN_TEST_RUNNER:?Set this to wasm-bindgen-test-runner 0.2.121 (matching Cargo.lock)}"
test -x "$WASM_BINDGEN_TEST_RUNNER"

# Do not reuse an unknown listener or stop another developer's process.
node -e 'const net = require("node:net"); (async () => { for (const port of [18107, 18109]) await new Promise((resolve, reject) => { const s = net.createServer(); s.once("error", reject); s.listen(port, "127.0.0.1", () => s.close(resolve)); }); })().catch(e => { console.error(e.message); process.exit(1); });'

z07_wasm="$(cargo test -p auki-sdk --lib --target wasm32-unknown-unknown --no-run --locked --message-format=json | node -e 'let data=""; process.stdin.on("data", c=>data+=c); process.stdin.on("end",()=>{ const files=data.split("\n").filter(Boolean).map(s=>JSON.parse(s)).filter(v=>v.reason==="compiler-artifact"&&v.target.name==="auki_sdk"&&v.profile.test).flatMap(v=>v.filenames).filter(f=>f.endsWith(".wasm")); if(files.length!==1) process.exit(1); process.stdout.write(files[0]); });')"
z07_artifacts="$z07_root/output/playwright/zitadel-z07-$$"
mkdir -p "$z07_artifacts"
cd "$z07_artifacts"
z07_session="zitadel-z07-$$"
z07_runner_pid=""
z07_fixture_pid=""
z07_browser_started=""
pw() { npx --yes --package @playwright/cli@0.1.19 playwright-cli --session "$z07_session" "$@"; }
cleanup() {
  if [[ -n "$z07_browser_started" ]]; then pw close >/dev/null 2>&1 || true; fi
  for z07_pid in "$z07_runner_pid" "$z07_fixture_pid"; do
    if [[ -n "$z07_pid" ]]; then kill "$z07_pid" 2>/dev/null || true; wait "$z07_pid" 2>/dev/null || true; fi
  done
}
trap cleanup EXIT INT TERM
wait_http() {
  node -e 'const url=process.argv[1]; (async()=>{const start=Date.now(); while(Date.now()-start<30000){try{if((await fetch(url,{signal:AbortSignal.timeout(1000)})).ok)return;}catch{} await new Promise(r=>setTimeout(r,100));} throw new Error("test server did not become ready");})().catch(e=>{console.error(e.message);process.exit(1);});' "$1"
}
node "$z07_root/test-support/zitadel-browser-fixture.mjs" > "$z07_artifacts/fixture.log" 2>&1 &
z07_fixture_pid=$!
wait_http http://127.0.0.1:18109/__stats
kill -0 "$z07_fixture_pid"

NO_HEADLESS=1 WASM_BINDGEN_TEST_ADDRESS=127.0.0.1:18107 "$WASM_BINDGEN_TEST_RUNNER" "$z07_wasm" --skip browser_suspension_resume > "$z07_artifacts/main-runner.log" 2>&1 &
z07_runner_pid=$!
wait_http http://127.0.0.1:18107/
kill -0 "$z07_runner_pid"
z07_browser_started=1
pw open http://127.0.0.1:18107/ --browser chrome
pw snapshot
pw run-code 'async (page) => { await page.waitForFunction(() => document.body.textContent.includes("test result:"), null, {timeout:45000}); const result = await page.locator("body").innerText(); const count = result.match(/test result: ok\. (\d+) passed; 0 failed; 0 ignored;/); if (!count || Number(count[1]) < 18) throw new Error(result); console.log(result); }'
pw snapshot

# Reload a separately filtered run so the harness can freeze the actual page
# after its signed near-expiry authority is installed, not during Wasm startup.
kill "$z07_runner_pid"
wait "$z07_runner_pid" 2>/dev/null || true
z07_runner_pid=""
NO_HEADLESS=1 WASM_BINDGEN_TEST_ADDRESS=127.0.0.1:18107 "$WASM_BINDGEN_TEST_RUNNER" "$z07_wasm" browser_suspension_resume > "$z07_artifacts/suspension-runner.log" 2>&1 &
z07_runner_pid=$!
wait_http http://127.0.0.1:18107/
kill -0 "$z07_runner_pid"
pw run-code 'async (page) => { const cdp = await page.context().newCDPSession(page); await cdp.send("Page.setWebLifecycleState", {state:"active"}); await page.goto("http://127.0.0.1:18107/"); await page.waitForFunction(() => globalThis.__z07Suspend === "ready"); await cdp.send("Page.setWebLifecycleState", {state:"frozen"}); try { await page.waitForTimeout(16000); } finally { await cdp.send("Page.setWebLifecycleState", {state:"active"}); } await page.evaluate(() => { globalThis.__z07Suspend = "resumed"; }); await page.waitForFunction(() => document.body.textContent.includes("test result:")); const result = await page.locator("body").innerText(); if (!result.includes("test result: ok. 1 passed; 0 failed; 0 ignored;")) throw new Error(result); console.log(result); }'
pw snapshot
printf 'Z07 real-browser checks passed; artifacts: %s\n' "$z07_artifacts"
