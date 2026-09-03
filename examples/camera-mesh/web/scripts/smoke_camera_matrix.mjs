import assert from "node:assert/strict";
import { spawn, execFile } from "node:child_process";
import { createHash } from "node:crypto";
import { EventEmitter, once } from "node:events";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { basename, dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";

const START_TIMEOUT_MS = 180_000;
const OPERATION_TIMEOUT_MS = parseTimeout(
  process.env.AUKI_CAMERA_MATRIX_OPERATION_TIMEOUT_MS,
  180_000,
);
const CLEANUP_TIMEOUT_MS = 120_000;
const COMMAND_OUTPUT_LIMIT_BYTES = 32 * 1024 * 1024;
const OUTPUT_LIMIT_BYTES = 128 * 1024;
const TEMP_PREFIX = "auki-camera-matrix-";
const CHECKS = ["info", "catalog", "registry", "stream"];
const BASE_PROTOCOL_IDS = [
  "/auki/auth/1/info/1.0.0",
  "/auki/auth/1/resources/0.3.0",
  "/auki/auth/1/resources/0.4.0",
  "/auki/auth/1/registries/0.3.0",
  "/auki/auth/1/blobs/0.1.0",
  "/auki/auth/1/message/0.1.0",
];
const STREAM_PROTOCOL_ID = "/auki/auth/1/stream/0.2.0";
const HANDLED_RATE_LIMIT_CONSOLE_ERROR =
  /^Failed to load resource: the server responded with a status of 429 \(\)$/;
const EDGES = [
  { id: "rust-to-web", publisher: "rust", viewer: "web" },
  { id: "rust-to-python", publisher: "rust", viewer: "python" },
  { id: "python-to-web", publisher: "python", viewer: "web" },
  { id: "python-to-rust", publisher: "python", viewer: "rust" },
  { id: "web-to-rust", publisher: "web", viewer: "rust" },
  { id: "web-to-python", publisher: "web", viewer: "python" },
];
const requestedEdgeId = process.argv
  .find((argument) => argument.startsWith("--edge="))
  ?.slice("--edge=".length);
const selectedEdges = requestedEdgeId
  ? EDGES.filter((edge) => edge.id === requestedEdgeId)
  : EDGES;
if (requestedEdgeId && selectedEdges.length === 0) {
  throw new Error(`unknown camera matrix edge: ${requestedEdgeId}`);
}

const webRoot = fileURLToPath(new URL("../", import.meta.url));
const cameraRoot = fileURLToPath(new URL("../../", import.meta.url));
const workspaceRoot = fileURLToPath(new URL("../../../../", import.meta.url));
const webDist = join(webRoot, "dist");
const pythonMain = join(cameraRoot, "python/main.py");
const pythonManifest = join(workspaceRoot, "bindings/python/auki-sdk-py/Cargo.toml");
const fixturePath = join(cameraRoot, "assets/deterministic-frame.jpg.base64");
const execFileAsync = promisify(execFile);
const headed = process.argv.includes("--headed");

if (process.argv.includes("--list")) {
  for (const edge of selectedEdges) {
    console.log(`${edge.id} ${edge.publisher} publisher -> ${edge.viewer} viewer`);
  }
  process.exit(0);
}

const credentials = requiredCredentials();
const childEnvironment = sanitizedEnvironment(process.env);
clearSensitiveEnvironment(process.env);
const deterministicSha256 = await deterministicFixtureSha256();
const processAgents = [];
const browserPeers = [];
let tempRoot;
let staticServer;
let browser;
let primaryFailure;
let matrixCompleted = false;

try {
  console.log("CAMERA_MATRIX_PHASE phase=build");
  tempRoot = await mkdtemp(join(tmpdir(), TEMP_PREFIX));
  await buildWebApp(childEnvironment);
  const nativeBinary = await buildNativeBinary(childEnvironment);
  const pythonBinary = await buildPythonEnvironment(childEnvironment, tempRoot);

  console.log("CAMERA_MATRIX_PHASE phase=start-process-peers");
  const rustPublisher = await startProcessPeer({
    label: "rust-publisher",
    runtime: "rust",
    role: "publisher",
    command: nativeBinary,
    args: [],
  });
  const rustViewer = await startProcessPeer({
    label: "rust-viewer",
    runtime: "rust",
    role: "viewer",
    command: nativeBinary,
    args: [],
  });
  const pythonPublisher = await startProcessPeer({
    label: "python-publisher",
    runtime: "python",
    role: "publisher",
    command: pythonBinary,
    args: ["-u", pythonMain],
  });
  const pythonViewer = await startProcessPeer({
    label: "python-viewer",
    runtime: "python",
    role: "viewer",
    command: pythonBinary,
    args: ["-u", pythonMain],
  });

  staticServer = await startStaticServer();
  const address = staticServer.address();
  assert(address && typeof address === "object");
  const browserUrl = `http://127.0.0.1:${address.port}/`;

  const { chromium } = await import("playwright");
  const launchOptions = { headless: !headed, env: childEnvironment };
  if (childEnvironment.AUKI_PLAYWRIGHT_CHANNEL) {
    launchOptions.channel = childEnvironment.AUKI_PLAYWRIGHT_CHANNEL;
  }
  browser = await chromium.launch(launchOptions);

  console.log("CAMERA_MATRIX_PHASE phase=start-browser-peers");
  const webPublisher = await createBrowserPeer(
    browser,
    browserUrl,
    "web-publisher",
    "publisher",
    credentials,
  );
  browserPeers.push(webPublisher);
  const webViewer = await createBrowserPeer(
    browser,
    browserUrl,
    "web-viewer",
    "viewer",
    credentials,
  );
  browserPeers.push(webViewer);

  const publishers = new Map([
    ["rust", rustPublisher],
    ["python", pythonPublisher],
    ["web", webPublisher],
  ]);
  const viewers = new Map([
    ["rust", rustViewer],
    ["python", pythonViewer],
    ["web", webViewer],
  ]);
  assertPeerSet([...publishers.values(), ...viewers.values()], credentials.domainId);

  console.log("CAMERA_MATRIX_PHASE phase=edges");
  const reports = new Map();
  for (const edge of selectedEdges) {
    const publisher = publishers.get(edge.publisher);
    const viewer = viewers.get(edge.viewer);
    assert(publisher && viewer);
    reports.set(
      edge.id,
      await runEdge(edge.id, publisher, viewer, deterministicSha256),
    );
  }

  if (!requestedEdgeId) {
    assert.equal(
      reports.get("rust-to-web").frameSha256,
      reports.get("rust-to-python").frameSha256,
      "Rust deterministic JPEG differed between Web and Python consumers",
    );
    assert.equal(
      reports.get("python-to-web").frameSha256,
      reports.get("python-to-rust").frameSha256,
      "Python deterministic JPEG differed between Web and Rust consumers",
    );
  }
  matrixCompleted = true;
} catch (error) {
  primaryFailure = redactError(error, credentials);
} finally {
  console.log("CAMERA_MATRIX_PHASE phase=cleanup");
  const cleanupErrors = [];
  for (const peer of browserPeers.reverse()) {
    await captureCleanup(cleanupErrors, peer.label, () =>
      withTimeout(peer.stop(), CLEANUP_TIMEOUT_MS, `stopping ${peer.label}`),
    );
  }
  for (const agent of processAgents.reverse()) {
    await captureCleanup(cleanupErrors, agent.label, () =>
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
    await captureCleanup(cleanupErrors, "temporary matrix state", () =>
      removeOwnedTempRoot(tempRoot),
    );
  }
  if (primaryFailure) {
    if (cleanupErrors.length) {
      primaryFailure.message += `\ncleanup also failed: ${cleanupErrors.join("; ")}`;
    }
    throw primaryFailure;
  }
  if (cleanupErrors.length) {
    throw new Error(`matrix cleanup failed: ${cleanupErrors.join("; ")}`);
  }
}

if (matrixCompleted) {
  console.log(
    `AUKI_CAMERA_MATRIX_OK peers=6 edges=${selectedEdges.length} protocols=6`,
  );
}

function parseTimeout(value, fallback) {
  if (value === undefined) return fallback;
  const parsed = Number(value);
  if (!Number.isSafeInteger(parsed) || parsed < 1_000) {
    throw new Error("AUKI_CAMERA_MATRIX_OPERATION_TIMEOUT_MS must be an integer >= 1000");
  }
  return parsed;
}

async function runEdge(id, publisher, viewer, fixtureSha256) {
  console.log(
    `CAMERA_EDGE_START id=${id} publisher=${publisher.label} viewer=${viewer.label}`,
  );

  const [, rejected] = await Promise.all([
    publisher.watchApproval(viewer.card.peerId, id),
    viewer.view(publisher.card, `${id}-pending`, 1, true),
  ]);
  assert.equal(rejected.ok, false, `${id} unexpectedly bypassed session approval`);
  assert.match(
    rejected.error ?? "",
    /approval[_ ]required|approval requested/i,
    `${id} returned the wrong pre-approval failure`,
  );

  await publisher.approve(viewer.card.peerId, `${id}-approve`);
  const view = await viewer.view(publisher.card, `${id}-view`, 2, false);
  assertViewReport(id, view, publisher.card.peerId);

  if (publisher.runtime !== "web") {
    assert.equal(
      view.frameSha256,
      fixtureSha256,
      `${id} did not decode the locked deterministic JPEG`,
    );
  }

  const [, pause] = await Promise.all([
    publisher.watchControl("pause", viewer.card.peerId, id),
    viewer.control("pause", publisher.card, `${id}-pause`),
  ]);
  assertControlResult(id, pause, "pause", publisher.card.peerId);

  const [, resume] = await Promise.all([
    publisher.watchControl("resume", viewer.card.peerId, id),
    viewer.control("resume", publisher.card, `${id}-resume`),
  ]);
  assertControlResult(id, resume, "resume", publisher.card.peerId);

  const requestId = `${id}-snapshot`;
  const expectedRequestId = viewer.runtime === "web" ? undefined : requestId;
  const [staged, snapshot] = await Promise.all([
    publisher.watchSnapshot(viewer.card.peerId, expectedRequestId, id),
    viewer.snapshot(
      publisher.card,
      requestId,
      `${id}-snapshot-command`,
    ),
  ]);
  assertSnapshotResult(id, snapshot, publisher.card.peerId, requestId);
  await publisher.assertSnapshot(snapshot, staged, viewer.card.peerId);
  if (publisher.runtime !== "web") {
    assert.equal(
      snapshot.sha256,
      fixtureSha256,
      `${id} snapshot differed from the deterministic publisher JPEG`,
    );
  }

  await viewer.disconnect();
  console.log(`CAMERA_EDGE_OK id=${id}`);
  return view;
}

function assertViewReport(id, report, targetPeerId) {
  assert.equal(report.ok, true, `${id} failed: ${report.error ?? "unknown error"}`);
  assert.equal(report.targetPeerId, targetPeerId, `${id} authenticated the wrong peer`);
  for (const check of CHECKS) {
    assert.equal(report.checks?.[check], true, `${id}/${check} did not pass`);
  }
  assert(report.frames >= 2, `${id} received only ${report.frames ?? 0} frames`);
  assert.match(report.frameSha256 ?? "", /^[0-9a-f]{64}$/, `${id} frame SHA-256`);
}

function assertControlResult(id, result, control, targetPeerId) {
  assert.equal(result.ok, true, `${id}/${control} failed: ${result.error ?? "unknown error"}`);
  assert.equal(result.targetPeerId, targetPeerId, `${id}/${control} reached the wrong peer`);
  assert.equal(
    normalizeControl(result.control ?? result.type),
    control,
    `${id}/${control} result type`,
  );
}

function assertSnapshotResult(id, result, targetPeerId, requestId) {
  assert.equal(result.ok, true, `${id}/snapshot failed: ${result.error ?? "unknown error"}`);
  assert.equal(result.targetPeerId, targetPeerId, `${id}/snapshot reached the wrong peer`);
  if (result.requestId !== undefined) {
    assert.equal(result.requestId, requestId, `${id}/snapshot request ID`);
  }
  assert.match(result.sha256 ?? "", /^[0-9a-f]{64}$/, `${id}/snapshot SHA-256`);
  assert(result.size > 0, `${id}/snapshot returned no bytes`);
}

async function startProcessPeer({ label, runtime, role, command, args }) {
  assert(tempRoot);
  const agent = createProcessAgent({
    label,
    runtime,
    role,
    command,
    args,
    cwd: workspaceRoot,
    environment: agentEnvironment(
      childEnvironment,
      join(tempRoot, `${label}.identity`),
      label,
      role,
    ),
  });
  processAgents.push(agent);
  agent.card = await withTimeout(agent.ready, START_TIMEOUT_MS, `starting ${label}`);
  return agent;
}

function createProcessAgent({ label, runtime, role, command, args, cwd, environment }) {
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
        if (history.length > 256) history.shift();
        events.emit("event", event);
      } catch {
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
  const state = { label, child, events, history, stdout, stderr };
  const ready = waitForAgentEvent(
    state,
    (event) => event.event === "ready",
    START_TIMEOUT_MS,
    `waiting for ${label} readiness`,
  ).then((event) => event.card);

  return {
    label,
    runtime,
    role,
    child,
    ready,
    result,
    card: undefined,
    watchApproval(peerId) {
      return waitForAgentEvent(
        state,
        (event) => event.event === "approval_required" && event.peerId === peerId,
        OPERATION_TIMEOUT_MS,
        `waiting for ${label} approval request from ${peerId}`,
      );
    },
    approve(peerId, id) {
      return commandResult(
        state,
        { command: "approve", id, peerId },
        (event) => event.event === "approve_result" && event.id === id,
        `waiting for ${label} approval result`,
      ).then((event) => {
        assert.equal(event.ok, true, `${label} approval failed: ${event.error ?? "unknown"}`);
        assert.equal(event.peerId, peerId, `${label} approved the wrong peer`);
      });
    },
    view(target, id, frames) {
      return commandResult(
        state,
        { command: "view", id, target, frames },
        (event) => event.event === "view_result" && event.id === id,
        `waiting for ${label} view result`,
      );
    },
    control(control, target, id) {
      return commandResult(
        state,
        { command: control, id, target },
        (event) => event.event === "control_result" && event.id === id,
        `waiting for ${label} ${control} result`,
      );
    },
    snapshot(target, requestId, id) {
      return commandResult(
        state,
        { command: "snapshot", id, target, requestId },
        (event) => event.event === "snapshot_result" && event.id === id,
        `waiting for ${label} snapshot result`,
      );
    },
    watchControl(control, peerId) {
      return waitForAgentEvent(
        state,
        (event) =>
          event.event === "control_received"
          && event.peerId === peerId
          && normalizeControl(event.control ?? event.type) === control,
        OPERATION_TIMEOUT_MS,
        `waiting for ${label} to receive ${control} from ${peerId}`,
      );
    },
    watchSnapshot(peerId, requestId) {
      return waitForAgentEvent(
        state,
        (event) =>
          event.event === "snapshot_staged"
          && event.peerId === peerId
          && (requestId === undefined || event.requestId === requestId),
        OPERATION_TIMEOUT_MS,
        `waiting for ${label} to stage snapshot ${requestId}`,
      );
    },
    async assertSnapshot(snapshot, staged, peerId) {
      assert.equal(staged.peerId, peerId, `${label} staged for the wrong viewer`);
      assert.equal(staged.sha256, snapshot.sha256, `${label} announced a different Blob hash`);
      assert.equal(staged.size, snapshot.size, `${label} announced a different Blob size`);
    },
    async disconnect() {
      // Process `view` consumes a fixed number of frames and closes the subscription.
    },
    stop() {
      if (stopping) return stopping;
      stopping = (async () => {
        if (child.exitCode === null && child.signalCode === null) {
          const id = `shutdown-${label}`;
          await commandResult(
            state,
            { command: "shutdown", id },
            (event) => event.event === "shutdown_ack" && event.id === id,
            `waiting for ${label} shutdown acknowledgement`,
            CLEANUP_TIMEOUT_MS,
          );
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
}

async function commandResult(
  state,
  command,
  predicate,
  description,
  timeoutMs = OPERATION_TIMEOUT_MS,
) {
  const waiting = waitForAgentEvent(state, predicate, timeoutMs, description);
  if (!state.child.stdin.write(`${JSON.stringify(command)}\n`)) {
    await once(state.child.stdin, "drain");
  }
  return waiting;
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
      () => finish(() => reject(new Error(
        `${description} timed out after ${timeoutMs}ms\n${agent.stdout.toString()}\n${agent.stderr.toString()}`,
      ))),
      timeoutMs,
    );
    agent.events.on("event", observed);
    agent.events.once("closed", closed);
    agent.events.once("parse_error", parseError);
  });
}

async function createBrowserPeer(browserInstance, url, label, role, input) {
  const context = await browserInstance.newContext();
  const page = await context.newPage();
  const consoleErrors = boundedOutput();
  let stopped = false;
  let activeTargetPeerId;

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
  await page.locator("#login-button").waitFor({ state: "visible", timeout: START_TIMEOUT_MS });
  await page.locator("#email").fill(input.email);
  await page.locator("#password").fill(input.password);
  await page.locator("#login-button").click();
  await page.locator("#peer-config").waitFor({ state: "visible", timeout: START_TIMEOUT_MS });

  const domainIds = await page.locator("#domain option").evaluateAll(
    (options) => options.map((option) => option.value),
  );
  assert(domainIds.includes(input.domainId), `${label} cannot access Domain ${input.domainId}`);
  await page.locator("#domain").selectOption(input.domainId);
  await page.locator("#role").selectOption(role);
  await page.locator(".advanced-field > summary").click();
  await page.locator("#display-name").fill(`Matrix ${label}`);
  await startBrowserPeerWithRetry(page, label);

  if (role === "publisher") {
    await page.locator("#capture-source").selectOption("synthetic");
    await page.locator("#publish-button").click();
    await waitForText(page, "#events", "Stream endpoint mounted", START_TIMEOUT_MS);
  }
  const card = JSON.parse(await browserElementText(page, "#local-card"));

  return {
    label,
    runtime: "web",
    role,
    card,
    watchApproval(peerId) {
      return page
        .locator("#pending-list li", { hasText: peerId })
        .locator("button", { hasText: "Allow" })
        .waitFor({ state: "visible", timeout: OPERATION_TIMEOUT_MS });
    },
    async approve(peerId) {
      const button = page
        .locator("#pending-list li", { hasText: peerId })
        .locator("button", { hasText: "Allow" });
      await button.waitFor({ state: "visible", timeout: OPERATION_TIMEOUT_MS });
      await button.click();
      await button.waitFor({ state: "detached", timeout: OPERATION_TIMEOUT_MS });
    },
    async view(target, _id, frames, expectPending) {
      activeTargetPeerId = target.peerId;
      const tile = browserCameraTile(page, target.peerId);
      if (await tile.count() === 0) await loadBrowserTarget(page, target);
      if (expectPending) {
        await waitForBrowserCameraStatus(
          page,
          target.peerId,
          "awaiting",
          OPERATION_TIMEOUT_MS,
        );
        return { ok: false, error: "approval requested" };
      }

      if (await tile.getAttribute("data-status") !== "live") {
        await tile.locator("button.tile-primary-action[data-action='retry']").click();
      }
      try {
        await waitFor(
          async () =>
            (await browserElementText(page, "#inspector-details")).includes(target.peerId)
            || await tile.getAttribute("data-status") === "error",
          OPERATION_TIMEOUT_MS,
          `${label} verified protocol metadata for ${target.peerId}`,
        );
        assert(
          (await browserElementText(page, "#inspector-details")).includes(target.peerId),
          `${label} camera connection returned before protocol metadata was verified`,
        );
        await waitFor(
          async () =>
            await receivedFrames(page, target.peerId) >= frames
            && await tile.locator("[data-role='remote-frame']")
              .evaluate((surface) => !surface.hidden
                && surface.width > 1
                && surface.height > 1
                && Number(surface.dataset.renderedRevision) > 0),
          OPERATION_TIMEOUT_MS,
          `${label} to decode ${frames} camera frames`,
        );
      } catch (error) {
        throw await browserOperationError(page, label, "view", error);
      }
      const inspector = JSON.parse(await browserElementText(page, "#inspector-details"));
      return {
        ok: true,
        targetPeerId: target.peerId,
        checks: browserInspectorChecks(inspector, target.peerId),
        frames: await receivedFrames(page, target.peerId),
        frameSha256: await browserImageSha256(
          page,
          `[data-camera-peer-id="${target.peerId}"] [data-role="remote-frame"]`,
        ),
      };
    },
    watchControl(control) {
      const phrase = control === "pause"
        ? "Camera paused by an approved viewer"
        : "Camera resumed by an approved viewer";
      return waitForNewText(page, "#events", phrase, OPERATION_TIMEOUT_MS);
    },
    async control(control, target) {
      const action = control === "pause" ? "source-pause" : "source-resume";
      const acknowledgement = `Message camera.${control} acknowledged`;
      const before = await occurrenceCount(page, "#events", acknowledgement);
      await browserCameraMenuAction(page, target.peerId, action);
      await waitFor(
        async () => await occurrenceCount(page, "#events", acknowledgement) > before,
        OPERATION_TIMEOUT_MS,
        `${label} ${control} acknowledgement`,
      );
      if (control === "pause") {
        await delay(800);
        const pausedAt = await receivedFrames(page, target.peerId);
        await delay(800);
        assert.equal(
          await receivedFrames(page, target.peerId),
          pausedAt,
          `${label} received frames while paused`,
        );
      } else {
        const resumedAt = await receivedFrames(page, target.peerId);
        await waitFor(
          async () => await receivedFrames(page, target.peerId) > resumedAt,
          OPERATION_TIMEOUT_MS,
          `${label} frames after resume`,
        );
      }
      return {
        ok: true,
        control,
        targetPeerId: target.peerId,
      };
    },
    watchSnapshot(peerId) {
      return waitForNewText(page, "#events", `staged for ${peerId}`, OPERATION_TIMEOUT_MS);
    },
    async snapshot(target, _requestId) {
      const before = await occurrenceCount(page, "#events", "fetched and SHA-256 verified");
      await browserCameraTile(page, target.peerId)
        .locator("button.tile-action[data-action='snapshot']")
        .click();
      await waitFor(
        async () => {
          const status = await browserElementText(page, "#snapshot-status");
          return /^[1-9][0-9]* bytes · [0-9a-f]{64} · /.test(status)
            && await occurrenceCount(page, "#events", "fetched and SHA-256 verified") > before;
        },
        OPERATION_TIMEOUT_MS,
        `${label} SHA-256-verified Blob snapshot`,
      );
      const status = await browserElementText(page, "#snapshot-status");
      const match = status.match(/^([1-9][0-9]*) bytes · ([0-9a-f]{64}) · /);
      assert(match);
      await page.locator("#close-snapshot-button").click();
      return {
        ok: true,
        targetPeerId: target.peerId,
        sha256: match[2],
        size: Number(match[1]),
      };
    },
    async assertSnapshot(snapshot, _observed, peerId) {
      await waitForText(
        page,
        "#events",
        `Blob ${snapshot.sha256} staged for ${peerId}`,
        OPERATION_TIMEOUT_MS,
      );
    },
    async disconnect() {
      if (role !== "viewer" || !activeTargetPeerId) return;
      const tile = browserCameraTile(page, activeTargetPeerId);
      if (await tile.count() === 0) return;
      await browserCameraMenuAction(page, activeTargetPeerId, "remove");
      await tile.waitFor({ state: "detached", timeout: OPERATION_TIMEOUT_MS });
      activeTargetPeerId = undefined;
    },
    async stop() {
      try {
        if (!stopped && !page.isClosed()) {
          await page.locator("dialog[open]").evaluateAll((dialogs) => {
            for (const openDialog of dialogs) openDialog.close();
          });
          const menu = page.locator(".session-menu");
          if (!(await menu.evaluate((element) => element.open))) {
            await menu.locator("summary").click();
          }
          await page.locator("#stop-peer-button").click();
          await waitForText(page, "#local-card", "Peer stopped", CLEANUP_TIMEOUT_MS);
          stopped = true;
        }
        const errors = consoleErrors.toString().trim();
        if (errors) throw new Error(`${label} console errors:\n${errors}`);
      } finally {
        await context.close();
      }
    },
  };
}

async function startBrowserPeerWithRetry(page, label) {
  const startButton = page.locator("#start-peer-button");
  const runtime = page.locator("#runtime-section");
  const attempts = 3;
  for (let attempt = 1; attempt <= attempts; attempt += 1) {
    await startButton.click();
    await waitFor(
      async () => await runtime.isVisible() || !(await startButton.isDisabled()),
      START_TIMEOUT_MS,
      `${label} startup attempt ${attempt}`,
    );
    if (await runtime.isVisible()) return;
    if (attempt < attempts) await delay(attempt * 1_000);
  }
  throw new Error(
    `${label} failed to start after ${attempts} attempts: ${await browserElementText(page, "#auth-error")}`,
  );
}

async function browserOperationError(page, label, operation, cause) {
  const values = await Promise.all(
    ["#events", "#metrics", "#inspector-details", "#snapshot-status"].map(async (selector) => {
      try {
        return `${selector}: ${(await browserElementText(page, selector)).slice(-4_000)}`;
      } catch {
        return `${selector}: unavailable`;
      }
    }),
  );
  const message = cause instanceof Error ? cause.message : String(cause);
  return new Error(`${label} ${operation} failed: ${message}\n${values.join("\n")}`);
}

async function loadBrowserTarget(page, card) {
  await page.locator("#add-camera-button").click();
  const details = page.locator("details.manual-target");
  if (!(await details.evaluate((element) => element.open))) {
    await details.locator("summary").click();
  }
  await page.locator("#manual-card").fill(JSON.stringify(card));
  await page.locator("#add-card-button").click();
  await browserCameraTile(page, card.peerId).waitFor({
    state: "visible",
    timeout: OPERATION_TIMEOUT_MS,
  });
}

function browserInspectorChecks(inspector, targetPeerId) {
  const catalog = inspector.catalog;
  const registry = inspector.registry;
  const manifest = inspector.stream?.manifest;
  return {
    info: inspector.info?.peerId === targetPeerId,
    catalog: catalog?.variant === "sensor_log"
      && catalog?.resource_id === "camera/main"
      && catalog?.source_peer_id === targetPeerId
      && catalog?.writer_peer_id === targetPeerId,
    registry: [registry?.sensor, registry?.clock, registry?.frame]
      .every((entry) => entry?.verified === true && /^[0-9a-f]{32}$/.test(entry.expectedHash)),
    stream: manifest?.payload === "camera_frame"
      && manifest?.resourceId === "camera/main",
  };
}

async function browserImageSha256(page, selector) {
  return page.locator(selector).evaluate(async (surface) => {
    const frameUrl = surface.dataset.frameUrl;
    if (!frameUrl) throw new Error("camera canvas has no retained source frame");
    const bytes = await (await fetch(frameUrl)).arrayBuffer();
    const digest = await crypto.subtle.digest("SHA-256", bytes);
    return [...new Uint8Array(digest)]
      .map((byte) => byte.toString(16).padStart(2, "0"))
      .join("");
  });
}

function assertPeerSet(peers, domainId) {
  const ids = new Set();
  for (const peer of peers) {
    const card = peer.card;
    assert.equal(card.version, 1, `${peer.label} card version`);
    assert.equal(card.domainId, domainId, `${peer.label} Domain`);
    assert.equal(card.peerId.length > 20, true, `${peer.label} Peer ID`);
    assert(card.routes.tcp.endsWith(`/p2p-circuit/p2p/${card.peerId}`));
    assert(card.routes.wss.includes("/wss/"));
    assert(card.routes.wss.endsWith(`/p2p-circuit/p2p/${card.peerId}`));
    for (const protocol of BASE_PROTOCOL_IDS) {
      assert(card.protocols.includes(protocol), `${peer.label} omitted ${protocol}`);
    }
    if (peer.role === "publisher") {
      assert(card.protocols.includes(STREAM_PROTOCOL_ID), `${peer.label} omitted Stream v2`);
    } else {
      assert(!card.protocols.includes(STREAM_PROTOCOL_ID), `${peer.label} advertised Stream v2`);
    }
    assert(!ids.has(card.peerId), `duplicate Peer ID ${card.peerId}`);
    ids.add(card.peerId);
  }
  assert.equal(ids.size, 6);
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

function agentEnvironment(environment, identityFile, nodeName, role) {
  return {
    ...environment,
    AUKI_EMAIL: credentials.email,
    AUKI_PASSWORD: credentials.password,
    AUKI_DOMAIN_ID: credentials.domainId,
    AUKI_IDENTITY_FILE: identityFile,
    AUKI_NODE_NAME: nodeName,
    AUKI_CAMERA_ROLE: role,
    AUKI_CAMERA_AUTO_APPROVE: "0",
    // Keep the matrix's byte-for-byte cross-consumer fixture assertions while
    // the interactive batch launcher uses the visibly animated feed.
    AUKI_CAMERA_FRAME_MODE: "still",
    AUKI_DISCOVERY_MODE: role === "publisher" ? "discover_and_advertise" : "discover_only",
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
      "auki-camera-mesh-native",
      "--message-format=json-render-diagnostics",
    ],
    { cwd: workspaceRoot, env: environment, maxBuffer: COMMAND_OUTPUT_LIMIT_BYTES },
  );
  for (const line of build.stdout.split(/\r?\n/)) {
    try {
      const message = JSON.parse(line);
      if (
        message.reason === "compiler-artifact"
        && message.target?.name === "auki-camera-mesh-native"
        && message.executable
      ) {
        return message.executable;
      }
    } catch {
      // Cargo may emit empty or informational lines around JSON diagnostics.
    }
  }
  throw new Error("Cargo did not report the native Camera Mesh executable");
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
  await execFileAsync(
    maturin.command,
    [...maturin.prefix, "develop", "--locked", "--manifest-path", pythonManifest],
    {
      cwd: workspaceRoot,
      env: { ...environment, VIRTUAL_ENV: venv },
      maxBuffer: COMMAND_OUTPUT_LIMIT_BYTES,
    },
  );
  return join(venv, "bin", "python");
}

async function resolveMaturin(environment) {
  if (await commandAvailable("maturin", environment)) return { command: "maturin", prefix: [] };
  if (await commandAvailable("uvx", environment)) return { command: "uvx", prefix: ["maturin"] };
  throw new Error("maturin is required (install maturin, or install uv so the runner can use uvx)");
}

async function commandAvailable(command, environment) {
  try {
    await execFileAsync(command, ["--version"], {
      env: environment,
      maxBuffer: 1024 * 1024,
    });
    return true;
  } catch {
    return false;
  }
}

async function deterministicFixtureSha256() {
  const encoded = (await readFile(fixturePath, "utf8")).trim();
  return createHash("sha256").update(Buffer.from(encoded, "base64")).digest("hex");
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

function browserCameraTile(page, peerId) {
  return page.locator(`[data-camera-peer-id="${peerId}"]`);
}

async function browserCameraMenuAction(page, peerId, action) {
  const tile = browserCameraTile(page, peerId);
  await tile.locator("button.tile-menu-trigger[data-action='menu']").click();
  const sheet = page.locator("#camera-actions-dialog");
  await sheet.waitFor({ state: "visible", timeout: OPERATION_TIMEOUT_MS });
  await sheet.locator(`button[data-camera-action="${action}"]`).click();
}

async function waitForBrowserCameraStatus(page, peerId, status, timeoutMs) {
  await waitFor(
    async () => await browserCameraTile(page, peerId).getAttribute("data-status") === status,
    timeoutMs,
    `camera ${peerId} status ${status}`,
  );
}

async function receivedFrames(page, peerId) {
  return Number(await browserCameraTile(page, peerId).getAttribute("data-frame-count") ?? "0");
}

async function browserElementText(page, selector) {
  return page.locator(selector).evaluate((element) => element.textContent ?? "");
}

async function waitForNewText(page, selector, text, timeoutMs) {
  const before = await occurrenceCount(page, selector, text);
  await waitFor(
    async () => await occurrenceCount(page, selector, text) > before,
    timeoutMs,
    `${selector} to receive ${JSON.stringify(text)}`,
  );
}

async function occurrenceCount(page, selector, text) {
  const value = (await browserElementText(page, selector)).toLowerCase();
  const needle = text.toLowerCase();
  return value.split(needle).length - 1;
}

async function waitForText(page, selector, text, timeoutMs) {
  await waitFor(
    async () => (await browserElementText(page, selector)).toLowerCase().includes(text.toLowerCase()),
    timeoutMs,
    `${selector} to contain ${JSON.stringify(text)}`,
  );
}

async function waitFor(predicate, timeoutMs, description) {
  const deadline = Date.now() + timeoutMs;
  let lastError;
  while (Date.now() < deadline) {
    try {
      if (await predicate()) return;
    } catch (error) {
      lastError = error;
    }
    await delay(200);
  }
  throw new Error(
    `timed out waiting for ${description}${lastError ? `: ${lastError}` : ""}`,
  );
}

function normalizeControl(value) {
  return String(value ?? "").replace(/^camera\./, "");
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
