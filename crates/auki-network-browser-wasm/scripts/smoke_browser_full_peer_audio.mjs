import { access, readFile } from "node:fs/promises";
import { spawn } from "node:child_process";
import http from "node:http";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright-core";
import { assertFreshPkgWeb } from "./smoke_freshness.mjs";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(__dirname, "..");
const sdkRoot = path.resolve(root, "../..");
const buildCommand =
  "wasm-pack build crates/auki-network-browser-wasm --target web --out-dir pkg-web --features browser_libp2p";

await assertFreshPkgWeb({
  artifact: path.join(root, "pkg-web/auki_network_browser_wasm_bg.wasm"),
  sources: [
    path.join(root, "src/lib.rs"),
    path.join(root, "src/browser_full_peer.rs"),
    path.join(root, "src/browser_audio.rs"),
    path.join(root, "src/browser_stream.rs"),
    path.join(root, "Cargo.toml"),
  ],
  buildCommand,
});

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
    const rel = url.pathname === "/" ? "/scripts/browser_full_peer_audio.html" : url.pathname;
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
    pageA.waitForFunction(() => typeof window.startAudioPeer === "function"),
    pageB.waitForFunction(() => typeof window.startAudioPeer === "function"),
  ]);

  const discoveryUrl = `inline-manager://${encodeURIComponent(`${managerAddr}|${relayAddr}`)}`;
  const [a, b] = await Promise.all([
    pageA.evaluate((args) => window.startAudioPeer(args), {
      seed: Array.from({ length: 32 }, (_, i) => i + 1),
      discoveryUrl,
      domainName: "browser-full-peer-audio",
      displayName: "Audio A",
    }),
    pageB.evaluate((args) => window.startAudioPeer(args), {
      seed: Array.from({ length: 32 }, (_, i) => i + 101),
      discoveryUrl,
      domainName: "browser-full-peer-audio",
      displayName: "Audio B",
    }),
  ]);
  if (!a.join?.ok || !b.join?.ok) throw new Error(`join failed: ${JSON.stringify({ a, b }, null, 2)}`);
  if (a.debug?.usesBrowserSession || b.debug?.usesBrowserSession) throw new Error("browser-session was used");
  if (!a.debug?.advertisedMultiaddrs?.length || !b.debug?.advertisedMultiaddrs?.length) {
    throw new Error("browser peers did not advertise multiaddrs");
  }

  const publish = await pageA.evaluate(() => window.publishGeneratedAudio());
  if (!publish?.ok) throw new Error(`generated audio publication failed: ${JSON.stringify(publish)}`);
  const subscribe = await pageB.evaluate((peerId) => window.subscribeToAudio(peerId), a.peerId);
  if (!subscribe?.ok) throw new Error(`audio subscription failed: ${JSON.stringify(subscribe)}`);

  try {
    await pageB.waitForFunction(
      () => {
        const evidence = window.audioEvidence();
        const media = evidence.mediaPresence;
        return evidence.debug?.usesBrowserSession === false &&
          media?.selectedRemoteStreamState === "connected" &&
          media?.playbackHealthy === true &&
          media?.lastFrameUnixMs !== null &&
          media?.outputLevel !== null;
      },
      null,
      { timeout: 10000 },
    );
  } catch (err) {
    const evidence = await pageB.evaluate(() => window.audioEvidence());
    throw new Error(`no received browser audio evidence: ${JSON.stringify(evidence, null, 2)}`, {
      cause: err,
    });
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
