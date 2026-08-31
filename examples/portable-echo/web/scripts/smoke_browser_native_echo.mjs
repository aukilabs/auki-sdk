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
const browserPage = join(webRoot, "scripts", "browser_echo.html");
const packageRoot = join(webRoot, "pkg-web");
const execFileAsync = promisify(execFile);

const credentials = requiredCredentials();
const childEnvironment = sanitizedEnvironment(process.env);
clearSensitiveEnvironment(process.env);
const { chromium } = await import("playwright");
const message = "hello from one shared Rust protocol";
let staticServer;
let browser;
let page;
let native;
let nativeStateDir;
let browserPeerStarted = false;

try {
  await buildWasmPackage(childEnvironment);
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
  page = await browser.newPage();
  await page.goto(`http://127.0.0.1:${address.port}/`, {
    waitUntil: "load",
  });
  await page.waitForFunction(() => typeof globalThis.echoHarness?.start === "function");

  const browserPeer = await withTimeout(
    page.evaluate(
      async ({ email, password, domainId }) =>
        globalThis.echoHarness.start({ email, password, domainId }),
      credentials,
    ),
    START_TIMEOUT_MS,
    "starting the browser peer",
  );
  browserPeerStarted = true;
  assert.equal(browserPeer.domainId, credentials.domainId);
  assert(browserPeer.wssRoute.includes("/wss/"));
  assert(browserPeer.tcpRoute, "the selected relay did not advertise a TCP route");
  assert(
    browserPeer.tcpRoute.endsWith(`/p2p-circuit/p2p/${browserPeer.peerId}`),
    "the browser TCP route does not target its Peer ID",
  );

  const serve = withTimeout(
    page.evaluate(() => globalThis.echoHarness.serveOnce()),
    ECHO_TIMEOUT_MS,
    "serving the browser echo request",
  );
  native = runNativeClient({
    binary: nativeBinary,
    browserPeerId: browserPeer.peerId,
    browserRoute: browserPeer.tcpRoute,
    environment: childEnvironment,
    stateDir: nativeStateDir,
    message,
  });
  const nativeCompletion = withTimeout(
    native.result,
    ECHO_TIMEOUT_MS,
    "waiting for the native peer to stop",
  );
  const [receipt, nativeResult] = await Promise.all([serve, nativeCompletion]);
  assertNativeSucceeded(nativeResult, browserPeer.peerId, message.length);

  const nativePeerId = extractRequiredLine(nativeResult.stdout, "PEER_ID");
  assert.equal(receipt.remotePeerId, nativePeerId);
  assert.equal(Buffer.from(receipt.payload).toString("utf8"), message);

  await withTimeout(
    page.evaluate(() => globalThis.echoHarness.shutdown()),
    START_TIMEOUT_MS,
    "shutting down the browser peer",
  );
  browserPeerStarted = false;

  console.log(
    `BROWSER_NATIVE_ECHO_OK browser_peer=${browserPeer.peerId} native_peer=${nativePeerId} bytes=${message.length}`,
  );
} catch (error) {
  throw redactError(error, credentials);
} finally {
  if (native) {
    await terminateChild(native.child);
  }
  if (page && browserPeerStarted) {
    await withTimeout(
      page.evaluate(() => globalThis.echoHarness.shutdown()),
      CLEANUP_TIMEOUT_MS,
      "cleaning up the browser peer",
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
  let file;
  if (pathname === "/") {
    file = browserPage;
  } else if (/^\/pkg-web\/[A-Za-z0-9_.-]+$/.test(pathname)) {
    file = join(packageRoot, pathname.slice("/pkg-web/".length));
  } else {
    response.writeHead(404);
    response.end();
    return;
  }

  const body = await readFile(file);
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

async function buildWasmPackage(environment) {
  await execFileAsync(
    "wasm-pack",
    [
      "build",
      ".",
      "--target",
      "web",
      "--out-dir",
      "pkg-web",
      "--dev",
      "--",
      "--locked",
    ],
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
        message.target?.name === "auki-portable-echo-native" &&
        message.executable
      ) {
        return message.executable;
      }
    } catch {
      // Cargo may emit an empty or non-JSON informational line.
    }
  }
  throw new Error("Cargo did not report the native echo executable");
}

function runNativeClient({
  binary,
  browserPeerId,
  browserRoute,
  environment,
  stateDir,
  message,
}) {
  const child = spawn(
    binary,
    [],
    {
      cwd: workspaceRoot,
      env: {
        ...environment,
        AUKI_EMAIL: credentials.email,
        AUKI_PASSWORD: credentials.password,
        AUKI_DOMAIN_ID: credentials.domainId,
        AUKI_ECHO_MESSAGE: message,
        AUKI_REMOTE_PEER_ID: browserPeerId,
        AUKI_REMOTE_ROUTE: browserRoute,
        AUKI_STATE_DIR: stateDir,
      },
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
  return { child, result };
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

function assertNativeSucceeded(result, browserPeerId, expectedBytes) {
  if (result.code !== 0) {
    throw new Error(
      `native peer exited with code ${result.code} signal ${result.signal ?? "none"}\n${result.stdout}\n${result.stderr}`,
    );
  }
  assert.match(result.stdout, /^STOPPED$/m);
  const echo = result.stdout.match(
    /^ECHO_OK remote_peer=(\S+) relayed=(\S+) bytes=(\d+)$/m,
  );
  assert(echo, `native peer did not report ECHO_OK\n${result.stdout}`);
  assert.equal(echo[1], browserPeerId);
  assert.equal(echo[2], "true");
  assert.equal(Number(echo[3]), expectedBytes);
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
  child.kill("SIGTERM");
  await Promise.race([closed, delay(2_000)]);
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
