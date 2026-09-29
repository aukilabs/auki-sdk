// Runs the Rust coordinator's regression suite inside Chromium; never contacts shared services.
import assert from "node:assert/strict";
import { execFileSync, spawn } from "node:child_process";
import { once } from "node:events";
import { chromium } from "playwright";
const runner = process.env.WASM_BINDGEN_TEST_RUNNER;
if (!runner)
  throw Error(
    "Set WASM_BINDGEN_TEST_RUNNER to wasm-bindgen-test-runner 0.2.121.",
  );
assert.match(
  execFileSync(runner, ["--version"], { encoding: "utf8" }),
  /0\.2\.121\b/,
);
const build = execFileSync(
  "cargo",
  [
    "test",
    "--locked",
    "-p",
    "auki-collaborative-mapping-web",
    "--lib",
    "--target",
    "wasm32-unknown-unknown",
    "--no-run",
    "--message-format=json",
  ],
  {
    cwd: new URL("../../../../..", import.meta.url),
    encoding: "utf8",
    maxBuffer: 20 * 1024 * 1024,
    stdio: ["ignore", "pipe", "inherit"],
  },
);
const files = build
  .split("\n")
  .filter(Boolean)
  .map((line) => JSON.parse(line))
  .filter(
    (item) =>
      item.reason === "compiler-artifact" &&
      item.target.name === "auki_collaborative_mapping_web" &&
      item.profile.test,
  )
  .flatMap((item) => item.filenames)
  .filter((file) => file.endsWith(".wasm"));
assert.equal(files.length, 1);
const port = 18147;
const server = spawn(runner, [files[0]], {
  env: {
    ...process.env,
    NO_HEADLESS: "1",
    WASM_BINDGEN_TEST_ADDRESS: `127.0.0.1:${port}`,
  },
  stdio: "pipe",
});
let logs = "";
server.stdout.on("data", (data) => {
  logs += data;
});
server.stderr.on("data", (data) => {
  logs += data;
});
let browser;
try {
  let ready = false;
  for (let i = 0; i < 300; i++) {
    if (server.exitCode !== null) throw Error(logs);
    try {
      ready = (await fetch(`http://127.0.0.1:${port}`)).ok;
    } catch {
      /* Waiting for runner. */
    }
    if (ready) break;
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  assert.ok(ready, logs);
  browser = await chromium.launch({ channel: "chrome", headless: true });
  const page = await browser.newPage();
  await page.route("**/*", (route) =>
    new URL(route.request().url()).hostname === "127.0.0.1"
      ? route.continue()
      : route.abort(),
  );
  await page.goto(`http://127.0.0.1:${port}`);
  await page.waitForFunction(
    () => document.body.textContent.includes("test result:"),
    null,
    { timeout: 60000 },
  );
  const result = await page.locator("body").innerText();
  console.log(result);
  assert.match(result, /test result: ok\. 13 passed; 0 failed/);
} finally {
  await browser?.close();
  if (server.exitCode === null) {
    server.kill("SIGTERM");
    await once(server, "exit");
  }
}
