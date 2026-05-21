import { access, readFile } from "node:fs/promises";
import http from "node:http";
import path from "node:path";
import { chromium } from "playwright-core";

const address = process.argv[2];
if (!address) {
  throw new Error("usage: node scripts/smoke_browser_join.mjs <manager-webrtc-direct-multiaddr>");
}

const advertised = process.argv.slice(3);
const root = path.resolve("crates/auki-network-browser-wasm");

const contentTypes = new Map([
  [".html", "text/html; charset=utf-8"],
  [".js", "text/javascript; charset=utf-8"],
  [".wasm", "application/wasm"],
  [".json", "application/json; charset=utf-8"],
]);

const server = http.createServer(async (req, res) => {
  try {
    const url = new URL(req.url, "http://127.0.0.1");
    const filePath = path.join(
      root,
      url.pathname === "/" ? "scripts/browser_join_smoke.html" : url.pathname,
    );
    const body = await readFile(filePath);
    res.setHeader("content-type", contentTypes.get(path.extname(filePath)) ?? "text/plain");
    res.end(body);
  } catch (err) {
    res.statusCode = 404;
    res.end(String(err));
  }
});

await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
const { port } = server.address();

const query = new URLSearchParams({ address });
for (const value of advertised) {
  query.append("advertised", value);
}

const executablePath = await chromeExecutable();
const browser = await chromium.launch({
  headless: true,
  ...(executablePath ? { executablePath } : {}),
});

try {
  const page = await browser.newPage();
  await page.goto(`http://127.0.0.1:${port}/scripts/browser_join_smoke.html?${query}`);
  await page.waitForFunction(() => document.body.dataset.result);
  const result = JSON.parse(await page.locator("body").getAttribute("data-result"));
  if (!result.ok) throw new Error(result.error);
  if (result.protocol !== "/auki/join/0.0.1") {
    throw new Error(`bad protocol: ${result.protocol}`);
  }
  if (!result.membership_json) {
    throw new Error("missing membership_json");
  }
  console.log(`ok ${result.local_peer_id} joined via ${result.manager_peer_id}`);
} finally {
  await browser.close();
  server.close();
}

async function chromeExecutable() {
  const candidates = [
    process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE,
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Chromium.app/Contents/MacOS/Chromium",
    "/usr/bin/google-chrome",
    "/usr/bin/chromium",
    "/usr/bin/chromium-browser",
  ].filter(Boolean);

  for (const candidate of candidates) {
    try {
      await access(candidate);
      return candidate;
    } catch {
      // Try the next common install path.
    }
  }

  return undefined;
}
