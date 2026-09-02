import assert from "node:assert/strict";
import { once } from "node:events";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { basename, dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { execFile, spawn } from "node:child_process";
import { promisify } from "node:util";

const START_TIMEOUT_MS = 120_000;
const ECHO_TIMEOUT_MS = 120_000;
const CLEANUP_TIMEOUT_MS = 10_000;
const OUTPUT_LIMIT_BYTES = 64 * 1024;
const COMMAND_OUTPUT_LIMIT_BYTES = 32 * 1024 * 1024;
const STATE_PREFIX = "auki-portable-echo-native-";
const webRoot = fileURLToPath(new URL("../", import.meta.url));
const workspaceRoot = fileURLToPath(new URL("../../../../", import.meta.url));
const webDist = join(webRoot, "dist");
const execFileAsync = promisify(execFile);

const credentials = requiredCredentials();
const childEnvironment = sanitizedEnvironment(process.env);
clearSensitiveEnvironment(process.env);
const { chromium } = await import("playwright");
const nativeMessage = "hello from one shared Rust protocol";
const browserToNativeMessage = "hello back from browser A";
let staticServer;
let browser;
let browserContext;
let firstPage;
let secondPage;
let native;
let nativeStateDir;
let browserStartups = [];
let firstPeerStarted = false;
let secondPeerStarted = false;

try {
  await buildWebApp(childEnvironment);
  const nativeBinary = await buildNativeBinary(childEnvironment);
  nativeStateDir = await mkdtemp(join(tmpdir(), STATE_PREFIX));
  staticServer = await startStaticServer();
  const address = staticServer.address();
  assert(address && typeof address === "object");

  const launchOptions = { env: childEnvironment, headless: true };
  if (childEnvironment.AUKI_PLAYWRIGHT_CHANNEL) {
    launchOptions.channel = childEnvironment.AUKI_PLAYWRIGHT_CHANNEL;
  }
  browser = await chromium.launch(launchOptions);
  browserContext = await browser.newContext();
  firstPage = await browserContext.newPage();
  secondPage = await browserContext.newPage();
  await Promise.all([
    firstPage.goto(`http://127.0.0.1:${address.port}/`, { waitUntil: "load" }),
    secondPage.goto(`http://127.0.0.1:${address.port}/`, { waitUntil: "load" }),
  ]);
  await Promise.all([
    firstPage.waitForFunction(() => !document.querySelector("#login-button")?.disabled),
    secondPage.waitForFunction(() => !document.querySelector("#login-button")?.disabled),
  ]);

  browserStartups = [
    startBrowserPeer(firstPage, credentials, () => { firstPeerStarted = true; }),
    startBrowserPeer(secondPage, credentials, () => { secondPeerStarted = true; }),
  ];
  const startupResults = await withTimeout(
    Promise.allSettled(browserStartups),
    START_TIMEOUT_MS,
    "starting two browser peers",
  );
  const [firstPeer, secondPeer] = requireStartedPeers(startupResults);
  assert.notEqual(firstPeer.peerId, secondPeer.peerId);
  assertBrowserPeer(firstPeer, credentials.domainId);
  assertBrowserPeer(secondPeer, credentials.domainId);

  await proveBrowserEcho(
    firstPage,
    firstPeer,
    secondPage,
    secondPeer,
    "hello from browser A",
  );
  await proveBrowserEcho(
    secondPage,
    secondPeer,
    firstPage,
    firstPeer,
    "hello from browser B",
  );

  assert(firstPeer.tcpRoute, "the required relay TCP route is missing");
  assert(
    firstPeer.tcpRoute.endsWith(`/p2p-circuit/p2p/${firstPeer.peerId}`),
    "the browser TCP route does not target its Peer ID",
  );

  native = runNativeClient({
    binary: nativeBinary,
    browserPeerId: firstPeer.peerId,
    environment: childEnvironment,
    stateDir: nativeStateDir,
    message: nativeMessage,
  });
  const nativeWaiting = waitForChildOutput(
    native,
    /^WAITING_FOR_PEER$/m,
    ECHO_TIMEOUT_MS,
    "waiting for the native peer to become bidirectional",
  );
  const nativeOutput = await nativeWaiting;
  assertNativeEchoOutput(nativeOutput, firstPeer.peerId, nativeMessage.length);

  const nativePeerId = extractRequiredLine(nativeOutput, "PEER_ID");
  await waitForLog(
    firstPage,
    `received from ${nativePeerId}: ${nativeMessage}`,
    "observing the native-to-browser echo",
  );

  await sendDiscoveredBrowserEcho(firstPage, nativePeerId, browserToNativeMessage);
  await waitForChildOutput(
    native,
    new RegExp(
      `^ECHO_SERVED remote_peer=${firstPeer.peerId} bytes=${browserToNativeMessage.length}$`,
      "m",
    ),
    ECHO_TIMEOUT_MS,
    "waiting for the native peer to report the browser echo",
  );

  native.child.kill("SIGINT");
  const nativeResult = await withTimeout(
    native.result,
    CLEANUP_TIMEOUT_MS,
    "stopping the native peer",
  );
  assertNativeSucceeded(
    nativeResult,
    firstPeer.peerId,
    nativeMessage.length,
    browserToNativeMessage.length,
  );

  await withTimeout(
    Promise.all([
      stopBrowserPeer(firstPage),
      stopBrowserPeer(secondPage),
    ]),
    START_TIMEOUT_MS,
    "shutting down the browser peers",
  );
  firstPeerStarted = false;
  secondPeerStarted = false;

  console.log(
    `PORTABLE_ECHO_MATRIX_OK browser_a=${firstPeer.peerId} browser_b=${secondPeer.peerId} native=${nativePeerId} exchange=dds-discovery directions=browser-a-to-browser-b,browser-b-to-browser-a,native-to-browser-a,browser-a-to-native`,
  );
} catch (error) {
  throw redactError(error, credentials);
} finally {
  if (native) {
    await terminateChild(native.child);
  }
  await withTimeout(
    Promise.allSettled(browserStartups),
    CLEANUP_TIMEOUT_MS,
    "settling browser peer startup before cleanup",
  ).catch(() => {});
  if (firstPage && firstPeerStarted) {
    await withTimeout(
      stopBrowserPeer(firstPage),
      CLEANUP_TIMEOUT_MS,
      "cleaning up browser peer A",
    ).catch(() => {});
  }
  if (secondPage && secondPeerStarted) {
    await withTimeout(
      stopBrowserPeer(secondPage),
      CLEANUP_TIMEOUT_MS,
      "cleaning up browser peer B",
    ).catch(() => {});
  }
  if (browser) {
    await withTimeout(browser.close(), CLEANUP_TIMEOUT_MS, "closing Chromium").catch(() => {});
  }
  if (staticServer) {
    staticServer.closeAllConnections();
    await new Promise((resolve) => staticServer.close(resolve));
  }
  if (nativeStateDir) {
    await removeOwnedStateDirectory(nativeStateDir);
  }
}

function requiredCredentials() {
  const names = ["AUKI_EMAIL", "AUKI_PASSWORD", "AUKI_DOMAIN_ID"];
  const missing = names.filter((name) => !process.env[name]);
  if (missing.length > 0) {
    throw new Error(`missing required environment: ${missing.join(", ")}`);
  }
  return {
    email: process.env.AUKI_EMAIL,
    password: process.env.AUKI_PASSWORD,
    domainId: process.env.AUKI_DOMAIN_ID,
  };
}

async function startBrowserPeer(page, input, peerStarted) {
  await page.fill("#email", input.email);
  await page.fill("#password", input.password);
  await page.click("#login-button");
  await page.waitForFunction(
    (domainId) =>
      Array.from(document.querySelector("#domain")?.options ?? [])
        .some((option) => option.value === domainId),
    input.domainId,
    { timeout: START_TIMEOUT_MS },
  );
  await page.selectOption("#domain", input.domainId);
  await page.selectOption("#discovery-mode", "discover_and_advertise");
  await page.click("#start-button");
  await page.waitForFunction(
    () => Boolean(document.querySelector("#local")?.dataset.peerId),
    undefined,
    { timeout: START_TIMEOUT_MS },
  );
  peerStarted();
  return page.$eval("#local", (local) => ({
    peerId: local.dataset.peerId,
    domainId: local.dataset.domainId,
    protocol: "/example/echo/1.0.0",
    wssRoute: local.dataset.wssRoute,
    tcpRoute: local.dataset.tcpRoute,
  }));
}

function requireStartedPeers(results) {
  const failure = results.find((result) => result.status === "rejected");
  if (failure) {
    throw failure.reason;
  }
  return results.map((result) => result.value);
}

function assertBrowserPeer(peer, expectedDomainId) {
  assert.equal(peer.domainId, expectedDomainId);
  assert.equal(peer.protocol, "/example/echo/1.0.0");
  assert(peer.wssRoute.includes("/wss/"));
  assert(
    peer.wssRoute.endsWith(`/p2p-circuit/p2p/${peer.peerId}`),
    "the browser WSS route does not target its Peer ID",
  );
}

async function proveBrowserEcho(senderPage, sender, receiverPage, receiver, message) {
  await Promise.all([
    waitForLog(
      receiverPage,
      `received from ${sender.peerId}: ${message}`,
      `observing ${sender.peerId} at ${receiver.peerId}`,
    ),
    sendDiscoveredBrowserEcho(senderPage, receiver.peerId, message),
  ]);
}

async function sendDiscoveredBrowserEcho(page, targetPeerId, message) {
  await selectDiscoveredPeer(page, targetPeerId);
  await page.fill("#message", message);
  const sent = waitForLog(
    page,
    `sent to ${targetPeerId}: ${message}`,
    `sending an echo to discovered peer ${targetPeerId}`,
  );
  await page.click("#send-button");
  await sent;
}

async function selectDiscoveredPeer(page, targetPeerId) {
  const deadline = Date.now() + ECHO_TIMEOUT_MS;
  while (Date.now() < deadline) {
    await page.waitForFunction(
      () => !document.querySelector("#refresh-button")?.disabled,
      undefined,
      { timeout: ECHO_TIMEOUT_MS },
    );
    await page.click("#refresh-button");
    await page.waitForFunction(
      () => !document.querySelector("#refresh-button")?.disabled,
      undefined,
      { timeout: ECHO_TIMEOUT_MS },
    );
    const found = await page.$eval(
      "#candidates",
      (select, expected) => Array.from(select.options).some((option) => option.value === expected),
      targetPeerId,
    );
    if (found) {
      await page.selectOption("#candidates", targetPeerId);
      await page.click("#use-candidate-button");
      const exactTarget = await page.$eval("#send", () => ({
        peerId: document.querySelector("#remote-peer")?.value,
        route: document.querySelector("#remote-route")?.value,
      }));
      assert.equal(exactTarget.peerId, targetPeerId);
      assert(exactTarget.route.includes("/wss/"));
      assert(exactTarget.route.endsWith(`/p2p-circuit/p2p/${targetPeerId}`));
      return;
    }
    await delay(500);
  }
  throw new Error(`browser did not discover Echo peer ${targetPeerId}`);
}

function waitForLog(page, line, description) {
  return withTimeout(
    page.waitForFunction(
      (expected) => document.querySelector("#log")?.textContent?.includes(expected),
      line,
      { timeout: ECHO_TIMEOUT_MS },
    ),
    ECHO_TIMEOUT_MS,
    description,
  );
}

async function stopBrowserPeer(page) {
  const running = await page.$eval("#local", (local) => Boolean(local.dataset.peerId));
  if (!running) {
    return;
  }
  await page.click("#stop-button");
  await page.waitForFunction(
    () => !document.querySelector("#local")?.dataset.peerId,
    undefined,
    { timeout: CLEANUP_TIMEOUT_MS },
  );
  const lastLogLine = await page.$eval("#log", (log) =>
    log.textContent?.trim().split("\n").at(-1),
  );
  assert.equal(lastLogLine, "Peer stopped");
}

function sanitizedEnvironment(source) {
  return Object.fromEntries(
    Object.entries(source).filter(([name]) => !isSensitiveEnvironmentName(name)),
  );
}

function clearSensitiveEnvironment(environment) {
  for (const name of Object.keys(environment)) {
    if (isSensitiveEnvironmentName(name)) {
      delete environment[name];
    }
  }
}

function isSensitiveEnvironmentName(name) {
  if (["AUKI_EMAIL", "DEBUG", "DEBUG_FILE", "PWDEBUG"].includes(name)) {
    return true;
  }
  return /(?:^|_)(?:PASSWORD|PASSWD|SECRET|TOKEN|ACCESS_KEY|API_KEY|APP_KEY|PRIVATE_KEY|CREDENTIALS?)(?:_|$)/i.test(
    name,
  );
}

async function startStaticServer() {
  const server = createServer((request, response) => {
    void serveStatic(request, response).catch(() => {
      if (!response.headersSent) {
        response.writeHead(500);
      }
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
  const contentType = pathname.endsWith(".wasm")
    ? "application/wasm"
    : pathname.endsWith(".js")
      ? "text/javascript; charset=utf-8"
      : "text/html; charset=utf-8";
  response.writeHead(200, {
    "Cache-Control": "no-store",
    "Content-Type": contentType,
  });
  response.end(body);
}

async function buildWebApp(environment) {
  await execFileAsync(
    "npm",
    ["run", "build"],
    {
      cwd: webRoot,
      env: environment,
      maxBuffer: COMMAND_OUTPUT_LIMIT_BYTES,
    },
  );
}

async function buildNativeBinary(environment) {
  const build = await execFileAsync(
    "cargo",
    [
      "build",
      "--locked",
      "-p",
      "auki-portable-echo-native",
      "--bin",
      "auki-portable-echo-interop",
      "--message-format=json-render-diagnostics",
    ],
    {
      cwd: workspaceRoot,
      env: environment,
      maxBuffer: COMMAND_OUTPUT_LIMIT_BYTES,
    },
  );
  for (const line of build.stdout.split(/\r?\n/)) {
    try {
      const message = JSON.parse(line);
      if (
        message.reason === "compiler-artifact" &&
        message.target?.name === "auki-portable-echo-interop" &&
        message.executable
      ) {
        return message.executable;
      }
    } catch {
      // Cargo may emit an empty or non-JSON informational line.
    }
  }
  throw new Error("Cargo did not report the native interop executable");
}

function runNativeClient({
  binary,
  browserPeerId,
  environment,
  stateDir,
  message,
}) {
  const nativeEnvironment = {
    ...environment,
    AUKI_EMAIL: credentials.email,
    AUKI_PASSWORD: credentials.password,
    AUKI_DOMAIN_ID: credentials.domainId,
    AUKI_DISCOVERY_MODE: "discover_and_advertise",
    AUKI_ECHO_MESSAGE: message,
    AUKI_KEEP_RUNNING: "1",
    AUKI_REMOTE_PEER_ID: browserPeerId,
    AUKI_STATE_DIR: stateDir,
  };
  delete nativeEnvironment.AUKI_REMOTE_ROUTE;
  const child = spawn(
    binary,
    [],
    {
      cwd: workspaceRoot,
      env: nativeEnvironment,
      stdio: ["ignore", "pipe", "pipe"],
    },
  );
  const stdout = boundedOutput();
  const stderr = boundedOutput();
  child.stdout.on("data", (chunk) => {
    stdout.append(chunk);
  });
  child.stderr.on("data", (chunk) => {
    stderr.append(chunk);
  });
  const result = new Promise((resolve, reject) => {
    child.once("error", reject);
    child.once("close", (code, signal) =>
      resolve({
        code,
        signal,
        stdout: stdout.toString(),
        stderr: stderr.toString(),
      }),
    );
  });
  return { child, result, stdout };
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
        const headBytes = Math.min(half - head.length, bytes.length);
        head = Buffer.concat([head, bytes.subarray(0, headBytes)]);
        bytes = bytes.subarray(headBytes);
      }
      if (bytes.length > 0) {
        tail = Buffer.concat([tail, bytes]);
        if (tail.length > half) {
          tail = tail.subarray(tail.length - half);
        }
      }
    },

    toString() {
      if (totalBytes <= OUTPUT_LIMIT_BYTES) {
        return Buffer.concat([head, tail]).toString("utf8");
      }
      return Buffer.concat([
        head,
        Buffer.from("\n... output truncated ...\n"),
        tail,
      ]).toString("utf8");
    },
  };
}

function assertNativeEchoOutput(output, browserPeerId, expectedBytes) {
  assert.match(
    output,
    new RegExp(`^DISCOVERY_SELECTED peer=${browserPeerId} routes=\\d+ expires=`, "m"),
  );
  assert.doesNotMatch(output, /^MANUAL_EXACT_TARGET /m);
  const echo = output.match(/^ECHO_OK remote_peer=(\S+) relayed=(\S+) bytes=(\d+)$/m);
  assert(echo, `native peer did not report ECHO_OK\n${output}`);
  assert.equal(echo[1], browserPeerId);
  assert.equal(echo[2], "true");
  assert.equal(Number(echo[3]), expectedBytes);
}

function assertNativeSucceeded(
  result,
  browserPeerId,
  outboundBytes,
  inboundBytes,
) {
  if (result.code !== 0) {
    throw new Error(
      `native peer exited with code ${result.code} signal ${result.signal ?? "none"}\n${result.stdout}\n${result.stderr}`,
    );
  }
  assert.match(result.stdout, /^STOPPED$/m);
  assertNativeEchoOutput(result.stdout, browserPeerId, outboundBytes);
  const served = result.stdout.match(
    /^ECHO_SERVED remote_peer=(\S+) bytes=(\d+)$/m,
  );
  assert(served, `native peer did not report ECHO_SERVED\n${result.stdout}`);
  assert.equal(served[1], browserPeerId);
  assert.equal(Number(served[2]), inboundBytes);
}

function waitForChildOutput(nativeProcess, pattern, timeoutMs, description) {
  return new Promise((resolve, reject) => {
    let settled = false;
    const cleanup = () => {
      clearTimeout(timer);
      nativeProcess.child.stdout.off("data", inspect);
      nativeProcess.child.off("close", closed);
    };
    const finish = (operation) => {
      if (settled) {
        return;
      }
      settled = true;
      cleanup();
      operation();
    };
    const inspect = () => {
      const output = nativeProcess.stdout.toString();
      if (pattern.test(output)) {
        finish(() => resolve(output));
      }
    };
    const closed = (code, signal) => {
      const output = nativeProcess.stdout.toString();
      finish(() =>
        reject(
          new Error(
            `${description}: native peer exited first (code=${code}, signal=${signal ?? "none"})\n${output}`,
          ),
        ),
      );
    };
    const timer = setTimeout(() => {
      finish(() => reject(new Error(`${description} timed out after ${timeoutMs}ms`)));
    }, timeoutMs);
    nativeProcess.child.stdout.on("data", inspect);
    nativeProcess.child.once("close", closed);
    inspect();
  });
}

function extractRequiredLine(output, name) {
  const match = output.match(new RegExp(`^${name}=(\\S+)$`, "m"));
  assert(match, `native peer did not report ${name}`);
  return match[1];
}

async function terminateChild(child) {
  if (child.exitCode !== null || child.signalCode !== null) {
    return;
  }
  let closed = once(child, "close").catch(() => {});
  child.kill("SIGINT");
  await Promise.race([closed, delay(2_000)]);
  if (child.exitCode === null && child.signalCode === null) {
    closed = once(child, "close").catch(() => {});
    child.kill("SIGTERM");
    await Promise.race([closed, delay(2_000)]);
  }
  if (child.exitCode === null && child.signalCode === null) {
    closed = once(child, "close").catch(() => {});
    child.kill("SIGKILL");
    await Promise.race([closed, delay(2_000)]);
  }
}

async function removeOwnedStateDirectory(path) {
  assert.equal(dirname(path), tmpdir());
  assert(basename(path).startsWith(STATE_PREFIX));
  await rm(path, { recursive: true, force: true, maxRetries: 2 });
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
