import { chromium } from "playwright";

const url = process.argv[2] ?? process.env.CAMERA_MESH_URL ?? "http://127.0.0.1:5173/";
const email = process.env.AUKI_EMAIL;
const password = process.env.AUKI_PASSWORD;
const requestedDomain = process.env.AUKI_DOMAIN_ID;
const cameraCount = Number(process.env.AUKI_CAMERA_WALL_COUNT ?? "2");
const timeout = 60_000;
const colocatedAdmissionBatch = 8;
const relayAdmissionWindowMs = 11_000;

if (!email || !password) {
  throw new Error("AUKI_EMAIL and AUKI_PASSWORD are required");
}
if (!Number.isInteger(cameraCount) || cameraCount < 2 || cameraCount > 16) {
  throw new Error("AUKI_CAMERA_WALL_COUNT must be an integer from 2 through 16");
}

const browser = await chromium.launch({
  headless: true,
  args: ["--use-fake-device-for-media-stream", "--use-fake-ui-for-media-stream"],
});
const contexts = [];
const publishers = [];
for (let index = 0; index < cameraCount; index += 1) {
  publishers.push(await newPeerPage());
}
const viewer = await newPeerPage();
const peers = [
  ...publishers.map((page, index) => [`publisher ${index + 1}`, page]),
  ["viewer", viewer],
];
const pageErrors = [];
const consoleErrors = [];

for (const [name, page] of peers) {
  page.on("pageerror", (error) => pageErrors.push(`${name}: ${error.message}`));
  page.on("console", (message) => {
    if (message.type() === "error") consoleErrors.push(`${name}: ${message.text()}`);
  });
}

try {
  const publisherDomains = [];
  for (const [index, publisher] of publishers.entries()) {
    publisherDomains.push(
      await loginAndStart(publisher, "publisher", `Camera ${index + 1}`),
    );
    process.stdout.write(`started publisher ${index + 1}/${cameraCount}\n`);
    // The smoke puts every Peer behind one IP. Respect the relay's admission
    // window between batches; real cameras normally arrive from distinct hosts.
    if ((index + 1) % colocatedAdmissionBatch === 0) {
      await delay(relayAdmissionWindowMs);
    }
  }
  const viewerDomain = await loginAndStart(viewer, "viewer", "Control room");
  process.stdout.write("started camera-wall viewer\n");
  if (publisherDomains.some((value) => value !== viewerDomain)) {
    throw new Error("browser peers selected different Domains");
  }
  await viewer.locator("button[data-column-count='4']").click();

  const viewerCard = await peerCard(viewer);
  const cards = [];
  for (const [index, publisher] of publishers.entries()) {
    cards.push(await startPublisher(publisher, index === 1 ? "webcam" : "synthetic"));
  }
  const cardA = cards[0];
  const cardB = cards[1];
  if (!cardA || !cardB) throw new Error("camera wall smoke requires two publishers");

  await requestThroughDiscovery(viewer, cardA.peerId);
  for (const card of cards.slice(1)) await requestThroughPeerCard(viewer, card);
  await Promise.all(publishers.map((publisher) => approve(publisher, viewerCard.peerId)));

  for (const card of cards) await connectApproved(viewer, card.peerId);
  await Promise.all(cards.map((card) => waitForFrames(viewer, card.peerId, 3)));
  await assertColumnLayout(viewer, 4, "desktop four-column wall");
  await Promise.all(cards.map((card) => assertStreamDiagnostics(viewer, card.peerId)));
  await assertInspector(viewer, cardA.peerId, "Smoke Camera 1 publisher");
  await assertInspector(viewer, cardB.peerId, "Smoke Camera 2 publisher");

  await cameraMenuAction(viewer, cardA.peerId, "source-pause");
  await waitForText(publishers[0], "#events", "Camera paused by an approved viewer", timeout);
  await delay(800);
  const pausedA = await receivedFrames(viewer, cardA.peerId);
  const runningB = await receivedFrames(viewer, cardB.peerId);
  await delay(1_000);
  if (await receivedFrames(viewer, cardA.peerId) !== pausedA) {
    throw new Error("pausing Camera A did not stop Camera A frames");
  }
  if (await receivedFrames(viewer, cardB.peerId) <= runningB) {
    throw new Error("pausing Camera A stopped Camera B");
  }

  await cameraMenuAction(viewer, cardA.peerId, "source-resume");
  await waitForText(publishers[0], "#events", "Camera resumed by an approved viewer", timeout);
  await waitFor(
    async () => await receivedFrames(viewer, cardA.peerId) > pausedA,
    timeout,
    "Camera A frames after resume",
  );

  await cameraTile(viewer, cardB.peerId)
    .locator("button.tile-action[data-action='snapshot']")
    .click();
  await viewer.locator("#snapshot-image").waitFor({ state: "visible", timeout });
  await waitFor(async () => {
    const status = await elementText(viewer, "#snapshot-status");
    const loaded = await viewer.locator("#snapshot-image").evaluate((image) => image.naturalWidth > 0);
    return loaded && /\b[0-9a-f]{64}\b/.test(status);
  }, timeout, "Camera B SHA-256-verified snapshot");
  await viewer.locator("#close-snapshot-button").click();

  await viewer.locator("button[data-column-count='3']").click();
  await assertColumnLayout(viewer, 3, "desktop three-column wall");
  if (await viewer.locator("#camera-grid > *").count() !== cameraCount + 1) {
    throw new Error("column layout did not render every camera plus one add-camera slot");
  }
  await Promise.all([
    waitForFrames(viewer, cardA.peerId, (await receivedFrames(viewer, cardA.peerId)) + 1),
    waitForFrames(viewer, cardB.peerId, (await receivedFrames(viewer, cardB.peerId)) + 1),
  ]);
  await assertResponsiveViewer(viewer, cardA.peerId, cardB.peerId);

  const beforeRemove = await receivedFrames(viewer, cardA.peerId);
  await cameraMenuAction(viewer, cardB.peerId, "remove");
  await cameraTile(viewer, cardB.peerId).waitFor({ state: "detached", timeout });
  await waitFor(
    async () => await receivedFrames(viewer, cardA.peerId) > beforeRemove,
    timeout,
    "Camera A after removing Camera B",
  );

  await publishers[0].locator("#stop-publish-button").click();
  await waitForText(publishers[0], "#events", "Camera publication stopped", timeout);
  await waitFor(
    async () => await cameraTile(viewer, cardA.peerId).getAttribute("data-status") === "ended",
    timeout,
    "Camera A offline tile",
  );
  await publishers[0].locator("#publish-button").click();
  await waitForText(publishers[0], "#events", "Stream endpoint mounted", timeout);
  await cameraTile(viewer, cardA.peerId)
    .locator("button.tile-primary-action[data-action='retry']")
    .click();
  await waitFor(
    async () => await cameraTile(viewer, cardA.peerId).getAttribute("data-status") === "awaiting",
    timeout,
    "Camera A session-scoped reapproval",
  );
  await approve(publishers[0], viewerCard.peerId);
  await connectApproved(viewer, cardA.peerId);
  await waitForFrames(viewer, cardA.peerId, 2);

  if (pageErrors.length) throw new Error(`browser page errors: ${pageErrors.join("; ")}`);
  if (consoleErrors.length) throw new Error(`browser console errors: ${consoleErrors.join("; ")}`);

  process.stdout.write(
    `camera wall smoke passed: one browser viewer kept ${cameraCount} concurrent camera streams independent in Domain ${viewerDomain}\n`,
  );
} catch (error) {
  await Promise.allSettled(peers.map(([name, page]) =>
    page.screenshot({
      path: `/tmp/auki-camera-mesh-${name.replaceAll(" ", "-")}.png`,
      fullPage: true,
    })));
  const diagnostics = await Promise.all(peers.map(async ([name, page]) =>
    `${name}:\n${await elementText(page, "#events").catch(() => "unavailable")}`));
  throw new Error(
    `${error instanceof Error ? error.message : String(error)}\n${diagnostics.join("\n")}\nconsole errors:\n${consoleErrors.join("\n")}`,
  );
} finally {
  await Promise.allSettled(peers.map(([, page]) => stopIfRunning(page)));
  await Promise.allSettled(contexts.map((context) => context.close()));
  await browser.close();
}

async function newPeerPage() {
  const context = await browser.newContext();
  await context.grantPermissions(["camera"], { origin: new URL(url).origin });
  contexts.push(context);
  return context.newPage();
}

async function loginAndStart(page, peerRole, label) {
  await page.goto(url, { waitUntil: "networkidle" });
  await page.locator("#login-button").waitFor({ state: "visible", timeout });
  await page.locator("#email").fill(email);
  await page.locator("#password").fill(password);
  await page.locator("#login-button").click();
  await page.locator("#peer-config").waitFor({ state: "visible", timeout });
  const domainIds = await page.locator("#domain option").evaluateAll(
    (options) => options.map((option) => option.value),
  );
  const selectedDomain = requestedDomain ?? domainIds[0];
  if (!selectedDomain || !domainIds.includes(selectedDomain)) {
    throw new Error(`requested Domain is unavailable for ${peerRole}`);
  }
  await page.locator("#domain").selectOption(selectedDomain);
  await page.locator("#role").selectOption(peerRole);
  await page.locator(".advanced-field > summary").click();
  await page.locator("#display-name").fill(`Smoke ${label} ${peerRole}`);
  await startPeerWithRetry(page, peerRole);
  return selectedDomain;
}

async function startPeerWithRetry(page, peerRole) {
  const startButton = page.locator("#start-peer-button");
  const runtime = page.locator("#runtime-section");
  const attempts = 3;
  for (let attempt = 1; attempt <= attempts; attempt += 1) {
    await startButton.click();
    await waitFor(
      async () => await runtime.isVisible() || !(await startButton.isDisabled()),
      timeout,
      `${peerRole} runtime startup attempt ${attempt}`,
    );
    if (await runtime.isVisible()) return;
    if (attempt < attempts) await delay(attempt * 1_000);
  }
  throw new Error(
    `${peerRole} failed to start after ${attempts} attempts: ${await elementText(page, "#auth-error")}`,
  );
}

async function startPublisher(page, capture) {
  await page.locator("#capture-source").selectOption(capture);
  await page.locator("#publish-button").click();
  await waitForText(page, "#events", "Stream endpoint mounted", timeout);
  if (capture === "webcam") {
    await waitForText(page, "#events", "Webcam permission granted", timeout);
  }
  return peerCard(page);
}

async function requestThroughDiscovery(page, peerId) {
  await page.locator("#add-camera-button").click();
  const result = page.locator(
    `#camera-results [data-candidate-peer-id="${peerId}"] button[data-candidate-peer-id]`,
  );
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    if (await result.count()) break;
    await waitFor(async () => !(await page.locator("#discover-button").isDisabled()), 20_000, "DDS scan");
    await page.locator("#discover-button").click();
    await delay(750);
  }
  if (!(await result.count())) throw new Error(`DDS did not return camera publisher ${peerId}`);
  await result.click();
  await waitForCameraStatus(page, peerId, "awaiting");
}

async function requestThroughPeerCard(page, card) {
  await page.locator("#add-camera-button").click();
  await page.locator(".manual-target > summary").click();
  await page.locator("#manual-card").fill(JSON.stringify(card));
  await page.locator("#add-card-button").click();
  await waitForText(page, "#events", "from a peer card", timeout);
  await waitForCameraStatus(page, card.peerId, "awaiting");
}

async function connectApproved(page, peerId) {
  const retry = cameraTile(page, peerId)
    .locator("button.tile-primary-action[data-action='retry']");
  await retry.waitFor({ state: "visible", timeout });
  await retry.click();
  await waitForCameraStatus(page, peerId, "live");
}

async function approve(page, peerId) {
  const control = page
    .locator("#pending-list li", { hasText: peerId })
    .locator("button", { hasText: "Allow" });
  await control.waitFor({ state: "visible", timeout });
  await control.click();
  await control.waitFor({ state: "detached", timeout });
}

async function assertInspector(page, peerId, expectedName) {
  await cameraTile(page, peerId).click({ position: { x: 10, y: 10 } });
  await waitFor(async () => {
    const value = await elementText(page, "#inspector-details");
    return value.includes(peerId) && value.includes('"verified": true');
  }, timeout, `verified protocol metadata for ${peerId}`);
  const value = await elementText(page, "#inspector-details");
  if (!value.includes(expectedName)) {
    throw new Error(`camera inspector did not identify ${expectedName}`);
  }
}

async function assertStreamDiagnostics(page, peerId) {
  await waitFor(async () => {
    const values = await cameraTile(page, peerId).evaluate((tile) => ({
      fps: Number(tile.dataset.streamFps),
      bandwidth: Number(tile.dataset.kibPerSecond),
      frameSize: Number(tile.dataset.averageFrameKib),
      frameAge: Number(tile.dataset.frameAgeMs),
    }));
    return values.fps > 0
      && values.bandwidth > 0
      && values.frameSize > 0
      && Number.isFinite(values.frameAge)
      && Math.abs(values.frameAge) < 60_000;
  }, timeout, `rolling Stream diagnostics for ${peerId}`);

  const summary = await cameraTile(page, peerId)
    .locator("[data-role='stream-diagnostics']")
    .textContent();
  if (!summary?.includes("fps") || !summary.includes("KiB/s") || !summary.includes("ms age")) {
    throw new Error(`camera ${peerId} does not render all three diagnostics: ${summary}`);
  }
}

async function assertResponsiveViewer(page, peerId, nextPeerId) {
  await page.setViewportSize({ width: 390, height: 844 });
  await waitFor(async () => await page.evaluate(() => document.documentElement.scrollWidth <= 391),
    timeout, "mobile layout without horizontal overflow");
  await assertColumnLayout(page, 2, "mobile column clamp");

  const layout = await page.evaluate(() => {
    const topbar = document.querySelector(".topbar")?.getBoundingClientRect();
    const add = document.querySelector("#add-camera-button")?.getBoundingClientRect();
    const tile = document.querySelector(".camera-tile")?.getBoundingClientRect();
    return {
      viewportWidth: window.innerWidth,
      topbarRight: topbar?.right,
      addWidth: add?.width,
      addHeight: add?.height,
      tileRight: tile?.right,
    };
  });
  if (
    layout.topbarRight === undefined
    || layout.topbarRight > layout.viewportWidth + 1
    || layout.tileRight === undefined
    || layout.tileRight > layout.viewportWidth + 1
    || (layout.addWidth ?? 0) < 44
    || (layout.addHeight ?? 0) < 44
  ) {
    throw new Error(`mobile Camera Mesh layout is invalid: ${JSON.stringify(layout)}`);
  }

  await cameraMenuAction(page, peerId, "details");
  const diagnostics = page.locator("#diagnostics-dialog");
  await diagnostics.waitFor({ state: "visible", timeout });
  const drawer = await diagnostics.boundingBox();
  if (!drawer || drawer.x < -1 || drawer.x + drawer.width > 391 || drawer.height > 845) {
    throw new Error(`mobile diagnostics drawer exceeds the viewport: ${JSON.stringify(drawer)}`);
  }
  for (const selector of ["#diagnostic-fps", "#diagnostic-bandwidth", "#diagnostic-frame-age"]) {
    const value = await elementText(page, selector);
    if (!value || value === "—") throw new Error(`${selector} did not expose a live value`);
  }
  if (!(await elementText(page, "#diagnostic-frame-size")).includes("KiB/frame")) {
    throw new Error("Diagnostics did not expose average JPEG size");
  }
  await page.locator("#close-diagnostics-button").click();

  await page.locator("button[data-column-count='1']").click();
  await assertColumnLayout(page, 1, "mobile focus mode");
  if (await page.locator("#camera-grid > .camera-tile").count() !== 1) {
    throw new Error("focus mode did not render exactly one camera");
  }
  const focused = cameraTile(page, peerId);
  const focusBox = await focused.boundingBox();
  if (!focusBox || focusBox.width < 370 || focusBox.height < 650) {
    throw new Error(`focused camera does not fill the mobile wall: ${JSON.stringify(focusBox)}`);
  }
  await page.locator("#next-camera-button").click();
  await cameraTile(page, nextPeerId).waitFor({ state: "visible", timeout });
  if (await elementText(page, "#focus-label") !== `2 / ${cameraCount}`) {
    throw new Error("focus navigation did not advance to the next camera");
  }
  await page.locator("#previous-camera-button").click();
  await cameraTile(page, peerId).waitFor({ state: "visible", timeout });

  await page.locator("button[data-column-count='2']").click();
  await assertColumnLayout(page, 2, "mobile two-column wall");
  await page.setViewportSize({ width: 1280, height: 720 });
  await page.locator("button[data-column-count='3']").click();
  await assertColumnLayout(page, 3, "restored desktop column preference");
}

async function assertColumnLayout(page, expected, label) {
  await waitFor(async () => {
    const tracks = await page.locator("#camera-grid").evaluate((grid) =>
      getComputedStyle(grid).gridTemplateColumns.split(/\s+/).filter(Boolean).length);
    const selected = await page.locator(`button[data-column-count='${expected}']`)
      .getAttribute("aria-pressed");
    return tracks === expected && selected === "true";
  }, timeout, label);
}

async function cameraMenuAction(page, peerId, action) {
  const tile = cameraTile(page, peerId);
  await tile.locator("button.tile-menu-trigger[data-action='menu']").click();
  const sheet = page.locator("#camera-actions-dialog");
  await sheet.waitFor({ state: "visible", timeout });
  await sheet.locator(`button[data-camera-action="${action}"]`).click();
}

async function waitForCameraStatus(page, peerId, status) {
  await waitFor(
    async () => await cameraTile(page, peerId).getAttribute("data-status") === status,
    timeout,
    `camera ${peerId} status ${status}`,
  );
}

async function waitForFrames(page, peerId, count) {
  await waitFor(
    async () => {
      const tile = cameraTile(page, peerId);
      const loaded = await tile.locator("[data-role='remote-frame']").evaluate(
        (image) => !image.hidden && image.naturalWidth > 0,
      );
      return loaded && await receivedFrames(page, peerId) >= count;
    },
    timeout,
    `${count} decoded frames from ${peerId}`,
  );
}

function cameraTile(page, peerId) {
  return page.locator(`[data-camera-peer-id="${peerId}"]`);
}

async function receivedFrames(page, peerId) {
  return Number(await cameraTile(page, peerId).getAttribute("data-frame-count") ?? "0");
}

async function peerCard(page) {
  return JSON.parse(await elementText(page, "#local-card"));
}

async function stopIfRunning(page) {
  if (page.isClosed()) return;
  const runtime = page.locator("#runtime-section");
  if (await runtime.count() === 0 || !(await runtime.isVisible())) return;
  if ((await elementText(page, "#local-card")).includes("Peer stopped")) return;
  const menu = page.locator(".session-menu");
  if (!(await menu.evaluate((element) => element.open))) {
    await menu.locator("summary").click();
  }
  await page.locator("#stop-peer-button").click({ timeout: 5_000 });
  await waitForText(page, "#local-card", "Peer stopped", 30_000);
}

async function elementText(page, selector) {
  return page.locator(selector).evaluate((element) => element.textContent ?? "");
}

async function waitForText(page, selector, text, timeoutMs) {
  await waitFor(
    async () => (await elementText(page, selector)).toLowerCase().includes(text.toLowerCase()),
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
  throw new Error(`timed out waiting for ${description}${lastError ? `: ${lastError}` : ""}`);
}

function delay(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}
