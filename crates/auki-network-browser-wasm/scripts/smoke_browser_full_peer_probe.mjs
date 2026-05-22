import { access, readFile } from "node:fs/promises";
import { spawn } from "node:child_process";
import http from "node:http";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright-core";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(__dirname, "..");
const sdkRoot = path.resolve(root, "../..");
const manager = spawn(
  "cargo",
  [
    "run",
    "-p",
    "auki-network",
    "--example",
    "browser_full_peer_manager",
    "--features",
    "browser_probe",
  ],
  {
    cwd: sdkRoot,
    stdio: ["ignore", "pipe", "pipe"],
  },
);
const relay = spawn(
  "cargo",
  [
    "run",
    "-p",
    "auki-network",
    "--example",
    "browser_full_peer_relay",
    "--features",
    "browser_probe",
  ],
  {
    cwd: sdkRoot,
    stdio: ["ignore", "pipe", "pipe"],
  },
);

let managerAddr = "";
let relayAddr = "";
let managerStderr = "";
manager.stdout.on("data", (chunk) => {
  const text = String(chunk);
  for (const match of text.matchAll(/manager_addr=(\S+)/g)) {
    managerAddr = preferLoopback(managerAddr, match[1]);
  }
  for (const match of text.matchAll(/relay_addr=(\S+)/g)) {
    relayAddr = preferLoopback(relayAddr, match[1]);
  }
});
manager.stderr.on("data", (chunk) => {
  managerStderr += String(chunk);
  process.stderr.write(chunk);
});
relay.stdout.on("data", (chunk) => {
  const text = String(chunk);
  for (const match of text.matchAll(/relay_addr=(\S+)/g)) {
    relayAddr = preferLoopback(relayAddr, match[1]);
  }
});
relay.stderr.on("data", (chunk) => {
  managerStderr += String(chunk);
  process.stderr.write(chunk);
});

const deadline = Date.now() + 60000;
while ((!managerAddr || !relayAddr) && Date.now() < deadline) {
  await new Promise((resolve) => setTimeout(resolve, 100));
}
if (!managerAddr || !relayAddr) {
  manager.kill("SIGTERM");
  relay.kill("SIGTERM");
  throw new Error(`manager did not print dialable manager and relay addresses\n${managerStderr}`);
}

const contentTypes = new Map([
  [".html", "text/html; charset=utf-8"],
  [".js", "text/javascript; charset=utf-8"],
  [".wasm", "application/wasm"],
  [".json", "application/json; charset=utf-8"],
]);

const server = http.createServer(async (req, res) => {
  try {
    const url = new URL(req.url, "http://127.0.0.1");
    const rel = url.pathname === "/" ? "/scripts/browser_full_peer_probe.html" : url.pathname;
    const file = path.join(root, rel);
    res.setHeader("Cross-Origin-Opener-Policy", "same-origin");
    res.setHeader("Cross-Origin-Embedder-Policy", "require-corp");
    res.setHeader("content-type", contentTypes.get(path.extname(file)) ?? "text/plain");
    res.end(await readFile(file));
  } catch (err) {
    res.statusCode = 404;
    res.end(String(err));
  }
});
server.listen(0, "127.0.0.1");
await new Promise((resolve) => server.once("listening", resolve));
const { port } = server.address();

const executablePath = await chromeExecutable();
const browser = await chromium.launch({
  headless: true,
  ...(executablePath ? { executablePath } : {}),
});
try {
  const pageA = await browser.newPage();
  const pageB = await browser.newPage();
  await pageA.goto(`http://127.0.0.1:${port}/`);
  await pageB.goto(`http://127.0.0.1:${port}/`);
  await Promise.all([
    pageA.waitForFunction(() => typeof window.runFullPeerProbe === "function"),
    pageB.waitForFunction(() => typeof window.runFullPeerProbe === "function"),
  ]);
  const seedA = Array.from({ length: 32 }, (_, i) => i + 1);
  const seedB = Array.from({ length: 32 }, (_, i) => i + 101);
  const discoveryUrl = `inline-manager://${encodeURIComponent(`${managerAddr}|${relayAddr}`)}`;
  const [a, b] = await Promise.all([
    pageA.evaluate((args) => window.runFullPeerProbe(args), {
      seed: seedA,
      discoveryUrl,
      domainName: "browser-full-peer",
    }),
    pageB.evaluate((args) => window.runFullPeerProbe(args), {
      seed: seedB,
      discoveryUrl,
      domainName: "browser-full-peer",
    }),
  ]);
  if (!a.join?.ok || !b.join?.ok) throw new Error(`join failed: ${JSON.stringify({ a, b }, null, 2)}`);
  if (a.debug?.usesBrowserSession || b.debug?.usesBrowserSession) throw new Error("browser-session was used");
  if (!a.debug?.advertisedMultiaddrs?.length || !b.debug?.advertisedMultiaddrs?.length) {
    throw new Error("browser peers did not advertise multiaddrs");
  }
  if (!a.snapshots.at(-1)?.participants?.some((p) => p.peerId === b.peerId)) {
    throw new Error("A cannot see B through full peer state");
  }
  if (!b.snapshots.at(-1)?.participants?.some((p) => p.peerId === a.peerId)) {
    throw new Error("B cannot see A through full peer state");
  }
} finally {
  await browser.close();
  server.close();
  manager.kill("SIGTERM");
  relay.kill("SIGTERM");
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

function preferLoopback(current, candidate) {
  if (!current) return candidate;
  if (candidate.includes("/ip4/127.0.0.1/")) return candidate;
  return current;
}
