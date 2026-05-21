import { access, readFile } from "node:fs/promises";
import http from "node:http";
import path from "node:path";
import { chromium } from "playwright-core";

const address = process.argv[2];
if (!address) {
  throw new Error(
    "usage: node scripts/smoke_browser_domain_join.mjs <manager-webrtc-direct-multiaddr>",
  );
}

const managerPeerId = address.match(/\/p2p\/([^/]+)$/)?.[1];
if (!managerPeerId) {
  throw new Error("manager multiaddr must end with /p2p/<peer-id>");
}

const domain = "browser-join-smoke";
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
    if (url.pathname === "/clusters") {
      res.setHeader("content-type", "application/json; charset=utf-8");
      res.end(
        JSON.stringify({
          clusters: [
            {
              name: domain,
              manager_peer_id: managerPeerId,
              manager_multiaddrs: [address],
              peer_count: 1,
              created_ns: 1,
              last_liveness_check_ns: 1,
            },
          ],
        }),
      );
      return;
    }

    const filePath = path.join(
      root,
      url.pathname === "/" ? "scripts/browser_domain_join_smoke.html" : url.pathname,
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

const query = new URLSearchParams({
  discovery: `http://127.0.0.1:${port}`,
  domain,
});

const executablePath = await chromeExecutable();
const browser = await chromium.launch({
  headless: true,
  ...(executablePath ? { executablePath } : {}),
});

try {
  const page = await browser.newPage();
  await page.goto(`http://127.0.0.1:${port}/scripts/browser_domain_join_smoke.html?${query}`);
  await page.waitForFunction(() => document.body.dataset.result);
  const result = JSON.parse(await page.locator("body").getAttribute("data-result"));
  if (!result.ok) throw new Error(result.error?.message ?? result.error ?? "join failed");
  console.log(`ok BrowserDomainSession.joinDomain joined ${domain} via ${managerPeerId}`);
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
