import { spawn } from "node:child_process";
import net from "node:net";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";

const reachabilityMessage =
  "Browser peer reachability failed through SDK networking. Do not add an Overwatch backend; fix SDK transport or add a generic Discovery signaling design.";

const here = path.dirname(fileURLToPath(import.meta.url));
const exampleRoot = path.resolve(here, "..");
const repoRoot = path.resolve(exampleRoot, "../..");
const discoveryRoot = "/Users/jb/Developer/Aukilabs/repos/discovery";
const discoveryPort = await findAvailablePort(8091);
const discoveryUrl = `http://127.0.0.1:${discoveryPort}`;
const appUrl = "http://127.0.0.1:7880";
const domainName = `overwatch-smoke-${Date.now()}`;

const processes = [];
const requestUrls = [];
const failedRequests = [];
const browserEvents = [];

try {
  await assertPath(discoveryRoot, "Discovery repo");
  start("cargo", ["run", "--", "--addr", `127.0.0.1:${discoveryPort}`], discoveryRoot, "discovery");
  await waitForHttp(`${discoveryUrl}/clusters`, "Discovery");

  start(
    "npm",
    ["--prefix", "examples/overwatch", "run", "dev", "--", "--host", "127.0.0.1", "--port", "7880"],
    repoRoot,
    "vite",
  );
  await waitForHttp(appUrl, "Vite");

  const browser = await chromium.launch({ headless: true });
  try {
    const [contextA, contextB] = await Promise.all([browser.newContext(), browser.newContext()]);
    const [pageA, pageB] = await Promise.all([contextA.newPage(), contextB.newPage()]);
    for (const page of [pageA, pageB]) {
      page.on("request", (request) => requestUrls.push(request.url()));
      page.on("requestfailed", (request) => failedRequests.push({
        url: request.url(),
        failure: request.failure()?.errorText ?? "unknown",
      }));
      page.on("console", (message) => browserEvents.push({
        type: message.type(),
        text: message.text(),
      }));
      page.on("pageerror", (error) => browserEvents.push({
        type: "pageerror",
        text: error.stack ?? error.message,
      }));
    }

    await enterDomain(pageA, "create");
    await waitForParticipants(pageA, 1);
    await enterDomain(pageB, "join");
    await Promise.all([waitForParticipants(pageA, 2), waitForParticipants(pageB, 2)]).catch(
      async (error) => {
        await assertNoAppApiRequests();
        throw new Error(`${reachabilityMessage}\n${await diagnostics(pageA, pageB)}\n${error.message}`);
      },
    );

    const snapshotB = await snapshot(pageB);
    const peerA = snapshotB.peers[0];
    const sensors = peerA
      ? await pageB.evaluate((peerId) => globalThis.__overwatchPark.sensors(peerId), peerA.peer_id)
      : [];
    const sensor = sensors.find((candidate) => candidate.kind === "camera") ?? sensors[0];
    if (!peerA || !sensor) {
      throw new Error(`${reachabilityMessage}\nPeer B did not discover Peer A's generated sensor.`);
    }
    const entry = await pageB.evaluate(
      ({ peerId, sensorId }) => globalThis.__overwatchPark.nextSensorFrame(peerId, sensorId),
      { peerId: peerA.peer_id, sensorId: sensor.sensor_id },
    );
    if (!entry || !Array.isArray(entry.payload) || entry.payload.length === 0) {
      throw new Error(`${reachabilityMessage}\nPeer B subscription did not receive a stream frame.`);
    }

    await assertNoAppApiRequests();
    console.log("Overwatch two-browser smoke passed.");
    await Promise.all([contextA.close(), contextB.close()]);
  } finally {
    await browser.close();
  }
} finally {
  await stopAll();
}

async function enterDomain(page, mode) {
  await page.goto(appUrl, { waitUntil: "domcontentloaded" });
  await page.getByLabel(/Discovery URL/i).fill(discoveryUrl);
  await page.getByLabel(/Domain name/i).fill(domainName);
  await page
    .getByRole("button", { name: mode === "create" ? /create new/i : /join existing/i })
    .click();
  await page.waitForFunction(
    () => globalThis.__overwatchPark?.snapshot()?.status?.source?.kind === "in_cluster",
    undefined,
    { timeout: 15_000 },
  );
}

async function waitForParticipants(page, count) {
  await page.waitForFunction(
    (expected) => {
      const snapshot = globalThis.__overwatchPark?.snapshot();
      return Number(Boolean(snapshot?.self)) + (snapshot?.peers?.length ?? 0) >= expected;
    },
    count,
    { timeout: 15_000 },
  );
}

async function snapshot(page) {
  return page.evaluate(() => globalThis.__overwatchPark?.snapshot());
}

async function diagnostics(pageA, pageB) {
  const [textA, textB, snapshotA, snapshotB] = await Promise.all([
    pageA.locator("body").innerText().catch((error) => `body unavailable: ${error.message}`),
    pageB.locator("body").innerText().catch((error) => `body unavailable: ${error.message}`),
    snapshot(pageA).catch(() => null),
    snapshot(pageB).catch(() => null),
  ]);
  return JSON.stringify(
    {
      pageA: { text: textA, snapshot: snapshotA },
      pageB: { text: textB, snapshot: snapshotB },
      recentRequests: requestUrls.slice(-50),
      failedRequests: failedRequests.slice(-20),
      browserEvents: browserEvents.slice(-50),
    },
    null,
    2,
  );
}

async function assertNoAppApiRequests() {
  const apiRequests = requestUrls.filter((url) => {
    try {
      return new URL(url).pathname.includes("/api/");
    } catch {
      return false;
    }
  });
  if (apiRequests.length > 0) {
    throw new Error(`Overwatch smoke made app backend requests: ${apiRequests.join(", ")}`);
  }
}

function start(command, args, cwd, name) {
  const child = spawn(command, args, { cwd, stdio: ["ignore", "pipe", "pipe"] });
  child.stdout.on("data", (chunk) => process.stdout.write(`[${name}] ${chunk}`));
  child.stderr.on("data", (chunk) => process.stderr.write(`[${name}] ${chunk}`));
  processes.push(child);
  child.once("exit", (code, signal) => {
    if (code && code !== 0) {
      process.stderr.write(`[${name}] exited with ${code}${signal ? ` (${signal})` : ""}\n`);
    }
  });
  return child;
}

async function stopAll() {
  for (const child of processes.splice(0).reverse()) {
    if (child.exitCode == null) {
      child.kill("SIGTERM");
    }
  }
  await delay(250);
}

async function waitForHttp(url, label) {
  const started = Date.now();
  while (Date.now() - started < 20_000) {
    try {
      const response = await fetch(url);
      if (response.ok) return;
    } catch {
      // retry until timeout
    }
    await delay(250);
  }
  throw new Error(`${label} did not become ready at ${url}`);
}

async function assertPath(target, label) {
  const fs = await import("node:fs/promises");
  try {
    await fs.stat(target);
  } catch {
    throw new Error(`${label} not found at ${target}`);
  }
}

function delay(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function findAvailablePort(startPort) {
  for (let port = startPort; port < startPort + 50; port += 1) {
    if (await canListen(port)) {
      return port;
    }
  }
  throw new Error(`No available port found starting at ${startPort}`);
}

function canListen(port) {
  return new Promise((resolve) => {
    const server = net.createServer();
    server.once("error", () => resolve(false));
    server.once("listening", () => {
      server.close(() => resolve(true));
    });
    server.listen(port, "127.0.0.1");
  });
}
