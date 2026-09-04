#!/usr/bin/env node

import { spawn } from "node:child_process";
import { access, mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const START_TIMEOUT_MS = 180_000;
const STOP_TIMEOUT_MS = 10_000;
const START_INTERVAL_MS = 1_250;
const START_BATCH_SIZE = 8;
const DDS_ADMISSION_COOLDOWN_MS = 11_000;
const MAX_PUBLISHERS = 16;
const TEMP_PREFIX = "auki-camera-publishers-";

const workspaceRoot = fileURLToPath(new URL("../../../", import.meta.url));
const pythonMain = fileURLToPath(new URL("../python/main.py", import.meta.url));
const pythonManifest = join(
  workspaceRoot,
  "bindings/python/auki-sdk-py/Cargo.toml",
);

let config;
const publishers = [];
let tempRoot;
let stopping = false;
let stopRequested = false;
let signalResolve;
let fatalReject;
const signal = new Promise((resolveSignal) => {
  signalResolve = resolveSignal;
});
const fatal = new Promise((_, reject) => {
  fatalReject = reject;
});
// A child can fail while the next staggered child is starting. Attach a
// handler immediately, before the main flow races this promise.
fatal.catch(() => {});

process.once("SIGINT", () => {
  stopRequested = true;
  signalResolve("SIGINT");
});
process.once("SIGTERM", () => {
  stopRequested = true;
  signalResolve("SIGTERM");
});

let failure;
try {
  config = readConfiguration();
  tempRoot = await mkdtemp(join(tmpdir(), TEMP_PREFIX));
  const nativeBinary = config.nativeCount > 0 ? await buildNative() : undefined;
  const pythonBinary =
    config.pythonCount > 0 ? await preparePythonBinding() : undefined;
  const specifications = publisherSpecifications(nativeBinary, pythonBinary);

  console.log(
    `Starting ${specifications.length} Camera Mesh publisher(s) in Domain ${config.domainId}...`,
  );
  let startingBatch = [];
  for (const [index, specification] of specifications.entries()) {
    const publisher = startPublisher(specification);
    publishers.push(publisher);
    startingBatch.push(publisher);
    publisher.ready.catch(() => {});

    const hasMore = index + 1 < specifications.length;
    const batchComplete = startingBatch.length === START_BATCH_SIZE || !hasMore;
    if (!batchComplete) {
      await Promise.race([delay(START_INTERVAL_MS), fatal, signal]);
      if (stopRequested) break;
      continue;
    }

    // Process startup time is variable, so spawn intervals alone do not bound
    // when children reach DDS. Wait until this whole admission batch is ready,
    // then let the colocated-IP rate-limit window clear before starting more.
    await Promise.race([
      Promise.all(startingBatch.map((candidate) => candidate.ready)),
      fatal,
      signal,
    ]);
    if (stopRequested) break;
    startingBatch = [];
    if (hasMore) {
      console.log(
        `${publishers.length} publishers ready; waiting ${DDS_ADMISSION_COOLDOWN_MS / 1_000}s for the DDS admission window...`,
      );
      await Promise.race([delay(DDS_ADMISSION_COOLDOWN_MS), fatal, signal]);
      if (stopRequested) break;
    }
  }

  await Promise.race([
    Promise.all(publishers.map((publisher) => publisher.ready)),
    fatal,
    signal,
  ]);
  if (
    !stopRequested &&
    publishers.length === specifications.length &&
    publishers.every((publisher) => publisher.card)
  ) {
    console.log(
      `All ${publishers.length} publishers are discoverable and auto-approve authenticated viewers in this Domain.`,
    );
    console.log("Press Ctrl-C to stop all publishers and discard their Peer IDs.");
    await Promise.race([fatal, signal]);
  }
} catch (error) {
  failure = error;
} finally {
  stopping = true;
  await stopPublishers();
  if (tempRoot) await removeOwnedTempRoot(tempRoot);
}

if (failure) {
  console.error(`Camera publisher launcher failed: ${errorText(failure)}`);
  process.exitCode = 1;
} else {
  console.log("All Camera Mesh publishers stopped.");
}

function readConfiguration() {
  const email = requiredEnvironment("AUKI_EMAIL");
  const password = requiredEnvironment("AUKI_PASSWORD");
  const domainId = requiredEnvironment("AUKI_DOMAIN_ID");
  const namePrefix = requiredEnvironment("AUKI_CAMERA_NAME_PREFIX").trim();
  if (!/^[A-Za-z0-9][A-Za-z0-9._-]{0,47}$/.test(namePrefix)) {
    throw new Error(
      "AUKI_CAMERA_NAME_PREFIX must be 1..48 letters, numbers, dots, underscores, or hyphens",
    );
  }
  const nativeCount = parseCount("AUKI_CAMERA_NATIVE_COUNT");
  const pythonCount = parseCount("AUKI_CAMERA_PYTHON_COUNT");
  const total = nativeCount + pythonCount;
  if (total < 1 || total > MAX_PUBLISHERS) {
    throw new Error(
      `native + Python publisher count must be between 1 and ${MAX_PUBLISHERS}`,
    );
  }
  return { email, password, domainId, namePrefix, nativeCount, pythonCount };
}

function requiredEnvironment(name) {
  const value = process.env[name];
  if (!value) throw new Error(`${name} is required`);
  return value;
}

function parseCount(name) {
  const value = requiredEnvironment(name);
  if (!/^(?:0|[1-9][0-9]*)$/.test(value)) {
    throw new Error(`${name} must be a non-negative integer`);
  }
  const count = Number(value);
  if (!Number.isSafeInteger(count) || count > MAX_PUBLISHERS) {
    throw new Error(`${name} must be between 0 and ${MAX_PUBLISHERS}`);
  }
  return count;
}

async function buildNative() {
  console.log("Building the native Camera Mesh publisher...");
  await run("cargo", ["build", "--locked", "-p", "auki-camera-mesh-native"]);
  const metadata = JSON.parse(
    await capture("cargo", ["metadata", "--format-version", "1", "--no-deps"]),
  );
  const executable = join(
    metadata.target_directory,
    "debug",
    process.platform === "win32"
      ? "auki-camera-mesh-native.exe"
      : "auki-camera-mesh-native",
  );
  await access(executable);
  return executable;
}

async function preparePythonBinding() {
  console.log("Preparing the local Python binding...");
  const metadata = JSON.parse(
    await capture("cargo", ["metadata", "--format-version", "1", "--no-deps"]),
  );
  const venv = join(metadata.target_directory, "camera-mesh-python-venv");
  const pythonCommand = process.env.AUKI_PYTHON || "python3";
  const pythonBinary = join(venv, "bin", "python");
  try {
    await access(pythonBinary);
  } catch {
    await run(pythonCommand, ["-m", "venv", venv]);
  }
  const maturin = await resolveMaturin();
  await run(
    maturin.command,
    [...maturin.prefix, "develop", "--locked", "--manifest-path", pythonManifest],
    { ...process.env, VIRTUAL_ENV: venv },
  );
  return pythonBinary;
}

async function resolveMaturin() {
  if (await commandAvailable("maturin")) {
    return { command: "maturin", prefix: [] };
  }
  if (await commandAvailable("uvx")) {
    return { command: "uvx", prefix: ["maturin"] };
  }
  throw new Error("Python publishers require maturin, or uv with uvx");
}

async function commandAvailable(command) {
  try {
    await capture(command, ["--version"]);
    return true;
  } catch {
    return false;
  }
}

function publisherSpecifications(nativeBinary, pythonBinary) {
  const specifications = [];
  const width = 2;
  for (let index = 1; index <= config.nativeCount; index += 1) {
    specifications.push({
      runtime: "native",
      name: `${config.namePrefix}-native-${String(index).padStart(width, "0")}`,
      command: nativeBinary,
      args: [],
    });
  }
  for (let index = 1; index <= config.pythonCount; index += 1) {
    specifications.push({
      runtime: "python",
      name: `${config.namePrefix}-python-${String(index).padStart(width, "0")}`,
      command: pythonBinary,
      args: ["-u", pythonMain],
    });
  }
  return specifications;
}

function startPublisher(specification) {
  if (!tempRoot || !specification.command) {
    throw new Error(`cannot start ${specification.name}: runtime is not prepared`);
  }
  const environment = {
    ...process.env,
    AUKI_EMAIL: config.email,
    AUKI_PASSWORD: config.password,
    AUKI_DOMAIN_ID: config.domainId,
    AUKI_IDENTITY_FILE: join(tempRoot, `${specification.name}.identity`),
    AUKI_NODE_NAME: specification.name,
    AUKI_CAMERA_ROLE: "publisher",
    AUKI_CAMERA_AUTO_APPROVE: "1",
    AUKI_DISCOVERY_MODE: "discover_and_advertise",
  };
  // The launcher is intentionally User-authenticated. Avoid an inherited App
  // credential pair making the SDK reject the otherwise valid configuration.
  delete environment.AUKI_APP_ACCESS_KEY;
  delete environment.AUKI_APP_SECRET;

  const child = spawn(specification.command, specification.args, {
    cwd: workspaceRoot,
    env: environment,
    stdio: ["pipe", "pipe", "pipe"],
  });
  let lineBuffer = "";
  let card;
  let readyResolve;
  let readyReject;
  const ready = withTimeout(
    new Promise((resolveReady, rejectReady) => {
      readyResolve = resolveReady;
      readyReject = rejectReady;
    }),
    START_TIMEOUT_MS,
    `waiting for ${specification.name}`,
  );
  const closed = new Promise((resolveClosed) => {
    child.once("close", (code, childSignal) => {
      resolveClosed({ code, signal: childSignal });
      if (stopping) return;
      const error = new Error(
        `${specification.name} exited unexpectedly (code=${code}, signal=${childSignal})`,
      );
      readyReject(error);
      fatalReject(error);
    });
  });

  child.once("error", (error) => {
    readyReject(error);
    if (!stopping) fatalReject(error);
  });
  child.stdout.setEncoding("utf8");
  child.stdout.on("data", (chunk) => {
    lineBuffer += chunk;
    const lines = lineBuffer.split(/\r?\n/);
    lineBuffer = lines.pop() || "";
    for (const line of lines) {
      if (!line) continue;
      try {
        const event = JSON.parse(line);
        if (event.event === "ready") {
          card = event.card;
          publisher.card = card;
          console.log(
            `[${specification.name}] ready peer=${card.peerId} runtime=${specification.runtime}`,
          );
          readyResolve(card);
        } else {
          console.log(`[${specification.name}] ${line}`);
        }
      } catch {
        console.log(`[${specification.name}] ${line}`);
      }
    }
  });
  child.stderr.setEncoding("utf8");
  child.stderr.on("data", (chunk) => {
    for (const line of chunk.trimEnd().split(/\r?\n/)) {
      if (line) console.error(`[${specification.name}] ${line}`);
    }
  });

  const publisher = { ...specification, child, ready, closed, card };
  return publisher;
}

async function stopPublishers() {
  if (publishers.length === 0) return;
  for (const [index, publisher] of publishers.entries()) {
    if (publisher.child.exitCode === null && publisher.child.stdin.writable) {
      publisher.child.stdin.write(
        `${JSON.stringify({ command: "shutdown", id: `launcher-${index}` })}\n`,
      );
    }
  }
  await Promise.all(
    publishers.map(async (publisher) => {
      const stopped = await settlesWithin(publisher.closed, STOP_TIMEOUT_MS);
      if (!stopped && publisher.child.exitCode === null) {
        publisher.child.kill("SIGTERM");
      }
      const terminated = stopped || (await settlesWithin(publisher.closed, 2_000));
      if (!terminated && publisher.child.exitCode === null) {
        publisher.child.kill("SIGKILL");
        await publisher.closed;
      }
    }),
  );
}

async function removeOwnedTempRoot(path) {
  const owned = resolve(path);
  if (dirname(owned) !== resolve(tmpdir()) || !basename(owned).startsWith(TEMP_PREFIX)) {
    throw new Error(`refusing to remove unowned launcher directory ${owned}`);
  }
  await rm(owned, { recursive: true, force: true });
}

function run(command, args, environment = process.env) {
  return new Promise((resolveRun, rejectRun) => {
    const child = spawn(command, args, {
      cwd: workspaceRoot,
      env: environment,
      stdio: "inherit",
    });
    child.once("error", rejectRun);
    child.once("close", (code, childSignal) => {
      if (code === 0) resolveRun();
      else {
        rejectRun(
          new Error(
            `${command} failed (code=${code}, signal=${childSignal || "none"})`,
          ),
        );
      }
    });
  });
}

function capture(command, args) {
  return new Promise((resolveCapture, rejectCapture) => {
    const child = spawn(command, args, {
      cwd: workspaceRoot,
      env: process.env,
      stdio: ["ignore", "pipe", "pipe"],
    });
    let stdout = "";
    let stderr = "";
    child.stdout.setEncoding("utf8");
    child.stderr.setEncoding("utf8");
    child.stdout.on("data", (chunk) => {
      stdout += chunk;
    });
    child.stderr.on("data", (chunk) => {
      stderr += chunk;
    });
    child.once("error", rejectCapture);
    child.once("close", (code) => {
      if (code === 0) resolveCapture(stdout);
      else rejectCapture(new Error(`${command} failed: ${stderr.trim()}`));
    });
  });
}

function withTimeout(promise, timeoutMs, description) {
  return Promise.race([
    promise,
    new Promise((_, reject) => {
      const timer = setTimeout(
        () => reject(new Error(`${description} timed out after ${timeoutMs}ms`)),
        timeoutMs,
      );
      timer.unref();
    }),
  ]);
}

async function settlesWithin(promise, timeoutMs) {
  return Promise.race([
    promise.then(() => true),
    delay(timeoutMs).then(() => false),
  ]);
}

function delay(milliseconds) {
  return new Promise((resolveDelay) => setTimeout(resolveDelay, milliseconds));
}

function errorText(error) {
  return error instanceof Error ? error.message : String(error);
}
