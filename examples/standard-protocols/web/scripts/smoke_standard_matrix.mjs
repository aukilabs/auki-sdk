import assert from "node:assert/strict";
import { spawn, execFile } from "node:child_process";
import { EventEmitter, once } from "node:events";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { basename, dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";

const START_TIMEOUT_MS = 180_000;
const DISCOVERY_TIMEOUT_MS = 180_000;
const PROBE_TIMEOUT_MS = 420_000;
const CLEANUP_TIMEOUT_MS = 120_000;
const COMMAND_OUTPUT_LIMIT_BYTES = 32 * 1024 * 1024;
const OUTPUT_LIMIT_BYTES = 128 * 1024;
const TEMP_PREFIX = "auki-standard-protocol-matrix-";
const HANDLED_RATE_LIMIT_CONSOLE_ERROR =
  /^Failed to load resource: the server responded with a status of 429 \(\)$/;
const PROTOCOLS = ["info", "catalog", "registry", "blob", "message", "stream"];
const INFO_PROTOCOL_ID = "/auki/auth/1/info/1.0.0";
const EDGES = [
  ["native-to-python", "native", "python"],
  ["python-to-native", "python", "native"],
  ["native-to-browser-a", "native", "browser-a"],
  ["browser-a-to-native", "browser-a", "native"],
  ["python-to-browser-b", "python", "browser-b"],
  ["browser-b-to-python", "browser-b", "python"],
  ["browser-a-to-browser-b", "browser-a", "browser-b"],
  ["browser-b-to-browser-a", "browser-b", "browser-a"],
];

const webRoot = fileURLToPath(new URL("../", import.meta.url));
const workspaceRoot = fileURLToPath(new URL("../../../../", import.meta.url));
const webDist = join(webRoot, "dist");
const pythonMain = join(workspaceRoot, "examples/standard-protocols/python/main.py");
const pythonManifest = join(workspaceRoot, "bindings/python/auki-sdk-py/Cargo.toml");
const execFileAsync = promisify(execFile);
const headed = process.argv.includes("--headed");

if (process.argv.includes("--list")) {
  for (const [id, source, target] of EDGES) {
    for (const protocol of PROTOCOLS) console.log(`${id}/${protocol} ${source} -> ${target}`);
  }
  process.exit(0);
}

const credentials = requiredCredentials();
const childEnvironment = sanitizedEnvironment(process.env);
clearSensitiveEnvironment(process.env);
let tempRoot;
let staticServer;
let browser;
let native;
let python;
const browserAgents = [];
let primaryFailure;
let matrixCompleted = false;

try {
  console.log("MATRIX_PHASE phase=build");
  tempRoot = await mkdtemp(join(tmpdir(), TEMP_PREFIX));
  await buildWebApp(childEnvironment);
  const [nativeBinary, pythonBinary] = await Promise.all([
    buildNativeBinary(childEnvironment),
    buildPythonEnvironment(childEnvironment, tempRoot),
  ]);
  staticServer = await startStaticServer();
  const address = staticServer.address();
  assert(address && typeof address === "object");

  console.log("MATRIX_PHASE phase=start-native-python");
  native = createProcessAgent({
    label: "native",
    command: nativeBinary,
    args: [],
    cwd: workspaceRoot,
    environment: agentEnvironment(childEnvironment, join(tempRoot, "native.identity"), "native-playground"),
  });
  python = createProcessAgent({
    label: "python",
    command: pythonBinary,
    args: ["-u", pythonMain],
    cwd: workspaceRoot,
    environment: agentEnvironment(childEnvironment, join(tempRoot, "python.identity"), "python-playground"),
  });
  const [nativeCard, pythonCard] = await withTimeout(
    Promise.all([native.ready, python.ready]),
    START_TIMEOUT_MS,
    "starting native and Python peers",
  );

  const { chromium } = await import("playwright");
  console.log("MATRIX_PHASE phase=start-browsers");
  const launchOptions = { headless: !headed, env: childEnvironment };
  if (childEnvironment.AUKI_PLAYWRIGHT_CHANNEL) {
    launchOptions.channel = childEnvironment.AUKI_PLAYWRIGHT_CHANNEL;
  }
  browser = await chromium.launch(launchOptions);
  const browserUrl = `http://127.0.0.1:${address.port}/`;
  const startBrowser = (label) => createBrowserAgent(
    browser,
    browserUrl,
    label,
    credentials,
    (agent) => browserAgents.push(agent),
  );
  const browserStarts = await Promise.allSettled([
    startBrowser("browser-a"),
    startBrowser("browser-b"),
  ]);
  const browserFailure = browserStarts.find((result) => result.status === "rejected");
  if (browserFailure) throw browserFailure.reason;
  const [browserA, browserB] = browserStarts.map((result) => result.value);

  const agents = new Map([
    ["native", {
      label: "native",
      card: nativeCard,
      discover: (protocol, id) => native.discover(protocol, id),
      probeDiscovered: (target, id) => native.probeDiscovered(target, id),
      probe: (target, id) => native.probe(target, id),
    }],
    ["python", {
      label: "python",
      card: pythonCard,
      discover: (protocol, id) => python.discover(protocol, id),
      probeDiscovered: (target, id) => python.probeDiscovered(target, id),
      probe: (target, id) => python.probe(target, id),
    }],
    ["browser-a", browserA],
    ["browser-b", browserB],
  ]);
  assertPeerSet(agents, credentials.domainId);

  console.log("MATRIX_PHASE phase=discover");
  const discoveredTargets = await assertDiscoveryMatrix(agents);

  console.log("MATRIX_PHASE phase=probe");
  let cases = 0;
  for (const [edgeId, sourceLabel, targetLabel] of EDGES) {
    const source = agents.get(sourceLabel);
    const target = agents.get(targetLabel);
    assert(source && target);
    const discovered = discoveredTargets.get(sourceLabel)?.get(target.card.peerId);
    assert(discovered, `${sourceLabel} did not retain its candidate for ${targetLabel}`);
    console.log(`MATRIX_EDGE_START id=${edgeId} source=${sourceLabel} target=${targetLabel}`);
    const result = await withTimeout(
      source.probeDiscovered(discovered, edgeId),
      PROBE_TIMEOUT_MS,
      `probing ${edgeId}`,
    );
    for (const protocol of PROTOCOLS) {
      if (result.checks?.[protocol] !== true) {
        throw new Error(
          `${edgeId}/${protocol} failed: ${result.errors?.[protocol] ?? "missing success result"}`,
        );
      }
      cases += 1;
      console.log(`CASE_OK id=${edgeId}/${protocol}`);
    }
    assert.equal(result.ok, true, `${edgeId} reported a failed aggregate result`);
  }
  assert.equal(cases, 48);
  matrixCompleted = true;
} catch (error) {
  primaryFailure = redactError(error, credentials);
} finally {
  console.log("MATRIX_PHASE phase=cleanup");
  const cleanupErrors = [];
  for (const agent of browserAgents.reverse()) {
    await captureCleanup(cleanupErrors, `${agent.label} peer`, () =>
      withTimeout(agent.stop(), CLEANUP_TIMEOUT_MS, `stopping ${agent.label}`),
    );
  }
  for (const agent of [python, native]) {
    if (!agent) continue;
    await captureCleanup(cleanupErrors, `${agent.label} peer`, () =>
      withTimeout(agent.stop(), CLEANUP_TIMEOUT_MS, `stopping ${agent.label}`),
    );
    await terminateChild(agent.child);
  }
  if (browser) {
    await captureCleanup(cleanupErrors, "Chromium", () =>
      withTimeout(browser.close(), CLEANUP_TIMEOUT_MS, "closing Chromium"),
    );
  }
  if (staticServer) {
    await captureCleanup(cleanupErrors, "static server", async () => {
      staticServer.closeAllConnections();
      await new Promise((resolve, reject) =>
        staticServer.close((error) => (error ? reject(error) : resolve())),
      );
    });
  }
  if (tempRoot) {
    await captureCleanup(cleanupErrors, "temporary matrix state", () => removeOwnedTempRoot(tempRoot));
  }
  if (primaryFailure) {
    if (cleanupErrors.length) {
      primaryFailure.message += `\ncleanup also failed: ${cleanupErrors.join("; ")}`;
    }
    throw primaryFailure;
  }
  if (cleanupErrors.length) throw new Error(`matrix cleanup failed: ${cleanupErrors.join("; ")}`);
}

if (matrixCompleted) {
  console.log("AUKI_PROTOCOL_MATRIX_OK peers=4 edges=8 protocols=6 cases=48");
}

function requiredCredentials() {
  const names = ["AUKI_EMAIL", "AUKI_PASSWORD", "AUKI_DOMAIN_ID"];
  const missing = names.filter((name) => !process.env[name]);
  if (missing.length) throw new Error(`missing required environment: ${missing.join(", ")}`);
  return {
    email: process.env.AUKI_EMAIL,
    password: process.env.AUKI_PASSWORD,
    domainId: process.env.AUKI_DOMAIN_ID,
  };
}

function agentEnvironment(environment, identityFile, nodeName) {
  return {
    ...environment,
    AUKI_EMAIL: credentials.email,
    AUKI_PASSWORD: credentials.password,
    AUKI_DOMAIN_ID: credentials.domainId,
    AUKI_IDENTITY_FILE: identityFile,
    AUKI_NODE_NAME: nodeName,
    AUKI_DISCOVERY_MODE: "discover_and_advertise",
  };
}

async function buildWebApp(environment) {
  await execFileAsync("npm", ["run", "build"], {
    cwd: webRoot,
    env: environment,
    maxBuffer: COMMAND_OUTPUT_LIMIT_BYTES,
  });
}

async function buildNativeBinary(environment) {
  const build = await execFileAsync(
    "cargo",
    [
      "build",
      "--locked",
      "-p",
      "auki-standard-protocols-native",
      "--message-format=json-render-diagnostics",
    ],
    { cwd: workspaceRoot, env: environment, maxBuffer: COMMAND_OUTPUT_LIMIT_BYTES },
  );
  for (const line of build.stdout.split(/\r?\n/)) {
    try {
      const message = JSON.parse(line);
      if (
        message.reason === "compiler-artifact" &&
        message.target?.name === "auki-standard-protocols-native" &&
        message.executable
      ) {
        return message.executable;
      }
    } catch {
      // Cargo may emit empty or informational lines around JSON diagnostics.
    }
  }
  throw new Error("Cargo did not report the native standard-protocol executable");
}

async function buildPythonEnvironment(environment, ownedTempRoot) {
  const venv = join(ownedTempRoot, "python-venv");
  const pythonCommand = environment.AUKI_PYTHON ?? "python3";
  await execFileAsync(pythonCommand, ["-m", "venv", venv], {
    cwd: workspaceRoot,
    env: environment,
    maxBuffer: COMMAND_OUTPUT_LIMIT_BYTES,
  });
  const maturin = await resolveMaturin(environment);
  await execFileAsync(maturin.command, [...maturin.prefix, "develop", "--manifest-path", pythonManifest], {
    cwd: workspaceRoot,
    env: { ...environment, VIRTUAL_ENV: venv },
    maxBuffer: COMMAND_OUTPUT_LIMIT_BYTES,
  });
  return join(venv, "bin", "python");
}

async function resolveMaturin(environment) {
  if (await commandAvailable("maturin", environment)) return { command: "maturin", prefix: [] };
  if (await commandAvailable("uvx", environment)) return { command: "uvx", prefix: ["maturin"] };
  throw new Error("maturin is required (install maturin, or install uv so the runner can use uvx)");
}

async function commandAvailable(command, environment) {
  try {
    await execFileAsync(command, ["--version"], { env: environment, maxBuffer: 1024 * 1024 });
    return true;
  } catch {
    return false;
  }
}

function createProcessAgent({ label, command, args, cwd, environment }) {
  const child = spawn(command, args, {
    cwd,
    env: environment,
    stdio: ["pipe", "pipe", "pipe"],
  });
  const events = new EventEmitter();
  const history = [];
  const stdout = boundedOutput();
  const stderr = boundedOutput();
  let lineBuffer = "";
  let stopping;

  child.stdout.setEncoding("utf8");
  child.stdout.on("data", (chunk) => {
    stdout.append(chunk);
    lineBuffer += chunk;
    const lines = lineBuffer.split(/\r?\n/);
    lineBuffer = lines.pop() ?? "";
    for (const line of lines) {
      if (!line) continue;
      try {
        const event = JSON.parse(line);
        history.push(event);
        if (history.length > 128) history.shift();
        events.emit("event", event);
      } catch (error) {
        events.emit("parse_error", new Error(`${label} emitted non-JSON stdout: ${line}`));
      }
    }
  });
  child.stderr.on("data", (chunk) => stderr.append(chunk));

  const result = new Promise((resolve, reject) => {
    child.once("error", reject);
    child.once("close", (code, signal) => {
      const value = { code, signal, stdout: stdout.toString(), stderr: stderr.toString() };
      events.emit("closed", value);
      resolve(value);
    });
  });

  const ready = waitForAgentEvent(
    { label, child, events, history, stdout, stderr },
    (event) => event.event === "ready",
    START_TIMEOUT_MS,
    `waiting for ${label} readiness`,
  ).then((event) => event.card);

  const agent = {
    label,
    child,
    ready,
    result,
    async discover(protocol, id) {
      const waiting = waitForAgentEvent(
        { label, child, events, history, stdout, stderr },
        (event) => event.event === "discovery_result" && event.id === id,
        DISCOVERY_TIMEOUT_MS,
        `waiting for ${label} discovery ${id}`,
      );
      child.stdin.write(`${JSON.stringify({ command: "discover", id, protocol })}\n`);
      const result = await waiting;
      if (result.ok !== true) {
        throw new Error(`${label} discovery failed: ${result.error ?? "unknown error"}`);
      }
      return result.candidates;
    },
    async probe(target, id) {
      const waiting = waitForAgentEvent(
        { label, child, events, history, stdout, stderr },
        (event) => event.event === "probe_result" && event.id === id,
        PROBE_TIMEOUT_MS,
        `waiting for ${label} probe ${id}`,
      );
      child.stdin.write(`${JSON.stringify({ command: "probe_all", id, target })}\n`);
      return waiting;
    },
    async probeDiscovered(target, id) {
      const waiting = waitForAgentEvent(
        { label, child, events, history, stdout, stderr },
        (event) => event.event === "probe_result" && event.id === id,
        PROBE_TIMEOUT_MS,
        `waiting for ${label} discovered probe ${id}`,
      );
      child.stdin.write(`${JSON.stringify({ command: "probe_discovered", id, target })}\n`);
      return waiting;
    },
    stop() {
      if (stopping) return stopping;
      stopping = (async () => {
        if (child.exitCode === null && child.signalCode === null) {
          const id = `shutdown-${label}`;
          const ack = waitForAgentEvent(
            { label, child, events, history, stdout, stderr },
            (event) => event.event === "shutdown_ack" && event.id === id,
            CLEANUP_TIMEOUT_MS,
            `waiting for ${label} shutdown acknowledgement`,
          );
          child.stdin.write(`${JSON.stringify({ command: "shutdown", id })}\n`);
          await ack;
        }
        const exited = await result;
        if (exited.code !== 0) {
          throw new Error(
            `${label} exited with code=${exited.code} signal=${exited.signal ?? "none"}\n${exited.stdout}\n${exited.stderr}`,
          );
        }
      })().catch(async (error) => {
        await terminateChild(child);
        throw error;
      });
      return stopping;
    },
  };
  return agent;
}

function waitForAgentEvent(agent, predicate, timeoutMs, description) {
  const existing = agent.history.find(predicate);
  if (existing) return Promise.resolve(existing);
  return new Promise((resolve, reject) => {
    let settled = false;
    const finish = (operation) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      agent.events.off("event", observed);
      agent.events.off("closed", closed);
      agent.events.off("parse_error", parseError);
      operation();
    };
    const observed = (event) => {
      if (predicate(event)) finish(() => resolve(event));
    };
    const closed = (result) => finish(() => reject(new Error(
      `${description}: ${agent.label} exited first (code=${result.code}, signal=${result.signal ?? "none"})\n${agent.stdout.toString()}\n${agent.stderr.toString()}`,
    )));
    const parseError = (error) => finish(() => reject(error));
    const timer = setTimeout(
      () => finish(() => reject(new Error(`${description} timed out after ${timeoutMs}ms`))),
      timeoutMs,
    );
    agent.events.on("event", observed);
    agent.events.once("closed", closed);
    agent.events.once("parse_error", parseError);
  });
}

async function createBrowserAgent(browserInstance, url, label, input, register) {
  const context = await browserInstance.newContext();
  let page;
  let card;
  const consoleErrors = boundedOutput();
  const agent = {
    label,
    get card() {
      return card;
    },
    discover(protocol) {
      return page.evaluate((protocolId) => window.aukiE2e.discover(protocolId), protocol);
    },
    probeDiscovered(target) {
      return page.evaluate((candidate) => window.aukiE2e.probeDiscovered(candidate), target);
    },
    async probe(target) {
      return page.evaluate((cardInput) => window.aukiE2e.probeAll(cardInput), target);
    },
    async stop() {
      try {
        if (page && !page.isClosed()) {
          const canStop = await page.evaluate(
            () => typeof window.aukiE2e?.stop === "function",
          ).catch(() => false);
          if (canStop) await page.evaluate(() => window.aukiE2e.stop());
        }
        const errors = consoleErrors.toString().trim();
        if (errors) throw new Error(`${label} console errors:\n${errors}`);
      } finally {
        await context.close();
      }
    },
  };
  register(agent);
  page = await context.newPage();
  page.on("console", (message) => {
    if (
      message.type() === "error"
      && !HANDLED_RATE_LIMIT_CONSOLE_ERROR.test(message.text())
    ) {
      consoleErrors.append(`${message.text()}\n`);
    }
  });
  page.on("pageerror", (error) => consoleErrors.append(`page error: ${error.message}\n`));
  await page.goto(url, { waitUntil: "load" });
  await page.waitForFunction(() => Boolean(window.aukiE2e), undefined, { timeout: START_TIMEOUT_MS });
  card = await withTimeout(
    page.evaluate((credentialsInput) => window.aukiE2e.start(credentialsInput), {
      email: input.email,
      password: input.password,
      domainId: input.domainId,
      label,
    }),
    START_TIMEOUT_MS,
    `starting ${label}`,
  );
  return agent;
}

async function assertDiscoveryMatrix(agents) {
  const deadline = Date.now() + DISCOVERY_TIMEOUT_MS;
  let lastMissing = [];
  let lastErrors = [];
  let round = 0;
  while (Date.now() < deadline) {
    round += 1;
    const observations = await Promise.all(
      [...agents].map(async ([label, agent]) => {
        try {
          return [
            label,
            await agent.discover(INFO_PROTOCOL_ID, `discover-info-${round}-${label}`),
            null,
          ];
        } catch (error) {
          return [label, [], error instanceof Error ? error.message : String(error)];
        }
      }),
    );
    lastMissing = [];
    lastErrors = [];
    for (const [observerLabel, candidates, error] of observations) {
      const observer = agents.get(observerLabel);
      assert(observer, `missing discovery observer ${observerLabel}`);
      if (error) lastErrors.push(`${observerLabel}: ${error}`);
      const byPeer = new Map(candidates.map((candidate) => [candidate.peerId, candidate]));
      assert(
        !byPeer.has(observer.card.peerId),
        `${observerLabel} discovery returned its own advertisement`,
      );
      for (const [targetLabel, target] of agents) {
        if (targetLabel === observerLabel) continue;
        const candidate = byPeer.get(target.card.peerId);
        if (!candidate) {
          lastMissing.push(`${observerLabel}->${targetLabel}`);
          continue;
        }
        const missingProtocols = target.card.protocols.filter(
          (protocol) => !candidate.servedProtocols.includes(protocol),
        );
        const hasLocalRoute = observer.card.runtime === "browser"
          ? candidate.routes.some((route) => route.includes("/wss/"))
          : candidate.routes.some(
            (route) => route.includes("/tcp/") && !route.includes("/wss/"),
          );
        if (missingProtocols.length || !hasLocalRoute) {
          lastMissing.push(`${observerLabel}->${targetLabel}/incomplete`);
          continue;
        }
        assert(
          candidate.servedProtocols.includes(INFO_PROTOCOL_ID),
          `${observerLabel}->${targetLabel} discovery omitted Info protocol`,
        );
        assert(
          candidate.routes.some((route) => route.endsWith(`/p2p/${target.card.peerId}`)),
          `${observerLabel}->${targetLabel} discovery returned no route for the expected Peer ID`,
        );
      }
    }
    if (!lastMissing.length) {
      const retained = new Map(
        observations.map(([label, candidates]) => [
          label,
          new Map(candidates.map((candidate) => [candidate.peerId, candidate])),
        ]),
      );
      await Promise.all(
        [...agents].map(([observerLabel, observer]) =>
          discoverAllWithRetry(observerLabel, observer, agents),
        ),
      );
      console.log("AUKI_DISCOVERY_MATRIX_OK peers=4 observations=12");
      return retained;
    }
    await delay(1_000);
  }
  throw new Error(
    `discovery matrix timed out; missing directed observations: ${lastMissing.join(", ")}; `
      + `last lookup errors: ${lastErrors.join("; ") || "none"}`,
  );
}

async function discoverAllWithRetry(observerLabel, observer, agents) {
  const deadline = Date.now() + DISCOVERY_TIMEOUT_MS;
  let attempt = 0;
  let lastFailure = "no lookup completed";
  while (Date.now() < deadline) {
    attempt += 1;
    try {
      const candidates = await observer.discover(
        undefined,
        `discover-all-${observerLabel}-${attempt}`,
      );
      const peerIds = new Set(candidates.map((candidate) => candidate.peerId));
      const missing = [...agents]
        .filter(([targetLabel, target]) =>
          targetLabel !== observerLabel && !peerIds.has(target.card.peerId))
        .map(([targetLabel]) => targetLabel);
      if (!missing.length) return;
      lastFailure = `missing ${missing.join(", ")}`;
    } catch (error) {
      lastFailure = error instanceof Error ? error.message : String(error);
    }
    await delay(1_000);
  }
  throw new Error(`${observerLabel} discover-all timed out: ${lastFailure}`);
}

function assertPeerSet(agents, expectedDomainId) {
  const peerIds = new Set();
  for (const agent of agents.values()) {
    const { card } = agent;
    assert.equal(card.version, 1, `${agent.label} card version`);
    assert.equal(card.domainId, expectedDomainId, `${agent.label} Domain`);
    assert.equal(card.protocols.length, 7, `${agent.label} wire protocol count`);
    assert(card.routes.tcp.endsWith(`/p2p-circuit/p2p/${card.peerId}`));
    assert(card.routes.wss.includes("/wss/"));
    assert(card.routes.wss.endsWith(`/p2p-circuit/p2p/${card.peerId}`));
    assert(!peerIds.has(card.peerId), `duplicate Peer ID ${card.peerId}`);
    peerIds.add(card.peerId);
  }
  assert.equal(peerIds.size, 4);
}

async function startStaticServer() {
  const server = createServer((request, response) => {
    void serveStatic(request, response).catch(() => {
      if (!response.headersSent) response.writeHead(500);
      response.end();
    });
  });
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  return server;
}

async function serveStatic(request, response) {
  const pathname = new URL(request.url ?? "/", "http://127.0.0.1").pathname;
  const relative = pathname === "/" ? "index.html" : pathname.slice(1);
  if (!/^(?:index\.html|assets\/[A-Za-z0-9_.-]+)$/.test(relative)) {
    response.writeHead(404);
    response.end();
    return;
  }
  const body = await readFile(join(webDist, relative));
  const contentType = relative.endsWith(".wasm")
    ? "application/wasm"
    : relative.endsWith(".js")
      ? "text/javascript; charset=utf-8"
      : "text/html; charset=utf-8";
  response.writeHead(200, { "Cache-Control": "no-store", "Content-Type": contentType });
  response.end(body);
}

function sanitizedEnvironment(source) {
  return Object.fromEntries(
    Object.entries(source).filter(([name]) => !isSensitiveEnvironmentName(name)),
  );
}

function clearSensitiveEnvironment(environment) {
  for (const name of Object.keys(environment)) {
    if (isSensitiveEnvironmentName(name)) delete environment[name];
  }
}

function isSensitiveEnvironmentName(name) {
  if (["AUKI_EMAIL", "DEBUG", "DEBUG_FILE", "PWDEBUG"].includes(name)) return true;
  return /(?:^|_)(?:PASSWORD|PASSWD|SECRET|TOKEN|ACCESS_KEY|API_KEY|APP_KEY|PRIVATE_KEY|CREDENTIALS?)(?:_|$)/i.test(name);
}

function boundedOutput() {
  let head = Buffer.alloc(0);
  let tail = Buffer.alloc(0);
  let totalBytes = 0;
  return {
    append(chunk) {
      let bytes = Buffer.from(chunk);
      totalBytes += bytes.length;
      const half = OUTPUT_LIMIT_BYTES / 2;
      if (head.length < half) {
        const length = Math.min(half - head.length, bytes.length);
        head = Buffer.concat([head, bytes.subarray(0, length)]);
        bytes = bytes.subarray(length);
      }
      if (bytes.length) {
        tail = Buffer.concat([tail, bytes]);
        if (tail.length > half) tail = tail.subarray(tail.length - half);
      }
    },
    toString() {
      if (totalBytes <= OUTPUT_LIMIT_BYTES) return Buffer.concat([head, tail]).toString("utf8");
      return Buffer.concat([head, Buffer.from("\n... output truncated ...\n"), tail]).toString("utf8");
    },
  };
}

async function terminateChild(child) {
  if (child.exitCode !== null || child.signalCode !== null) return;
  let closed = once(child, "close").catch(() => undefined);
  child.kill("SIGTERM");
  await Promise.race([closed, delay(2_000)]);
  if (child.exitCode === null && child.signalCode === null) {
    closed = once(child, "close").catch(() => undefined);
    child.kill("SIGKILL");
    await Promise.race([closed, delay(2_000)]);
  }
}

async function removeOwnedTempRoot(path) {
  assert.equal(dirname(path), tmpdir());
  assert(basename(path).startsWith(TEMP_PREFIX));
  await rm(path, { recursive: true, force: true, maxRetries: 2 });
}

async function captureCleanup(errors, name, cleanup) {
  try {
    await cleanup();
  } catch (error) {
    errors.push(`${name}: ${error instanceof Error ? error.message : String(error)}`);
  }
}

function withTimeout(promise, timeoutMs, description) {
  let timer;
  const timeout = new Promise((_, reject) => {
    timer = setTimeout(
      () => reject(new Error(`${description} timed out after ${timeoutMs}ms`)),
      timeoutMs,
    );
  });
  return Promise.race([promise, timeout]).finally(() => clearTimeout(timer));
}

function delay(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

function redactError(error, secretValues) {
  const message = error instanceof Error ? error.stack ?? error.message : String(error);
  const redacted = [secretValues.email, secretValues.password]
    .filter(Boolean)
    .sort((left, right) => right.length - left.length)
    .reduce((text, secret) => text.replaceAll(secret, "[redacted]"), message);
  return new Error(redacted);
}
