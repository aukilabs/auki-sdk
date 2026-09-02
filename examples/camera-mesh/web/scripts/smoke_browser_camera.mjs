import { chromium } from "playwright";

const url = process.argv[2] ?? process.env.CAMERA_MESH_URL ?? "http://127.0.0.1:5173/";
const email = process.env.AUKI_EMAIL;
const password = process.env.AUKI_PASSWORD;
const requestedDomain = process.env.AUKI_DOMAIN_ID;

if (!email || !password) {
  throw new Error("AUKI_EMAIL and AUKI_PASSWORD are required");
}

const browser = await chromium.launch({
  headless: true,
  args: ["--use-fake-device-for-media-stream", "--use-fake-ui-for-media-stream"],
});
const context = await browser.newContext();
await context.grantPermissions(["camera"], { origin: new URL(url).origin });
const firstTab = await context.newPage();
const secondTab = await context.newPage();
const pageErrors = [];
const consoleErrors = [];

for (const [name, page] of [["first tab", firstTab], ["second tab", secondTab]]) {
  page.on("pageerror", (error) => pageErrors.push(`${name}: ${error.message}`));
  page.on("console", (message) => {
    if (message.type() === "error") consoleErrors.push(`${name}: ${message.text()}`);
  });
}

try {
  const forward = await runCameraFlow(firstTab, secondTab, {
    label: "forward",
    capture: "synthetic",
    target: "discovery",
  });
  const reverse = await runCameraFlow(secondTab, firstTab, {
    label: "reverse",
    capture: "webcam",
    target: "peer-card",
  });

  if (pageErrors.length) throw new Error(`browser page errors: ${pageErrors.join("; ")}`);
  if (consoleErrors.length) throw new Error(`browser console errors: ${consoleErrors.join("; ")}`);

  process.stdout.write(
    `camera mesh smoke passed in both directions: ${forward.publisherPeerId} -> ${forward.viewerPeerId}, then ${reverse.publisherPeerId} -> ${reverse.viewerPeerId} in Domain ${forward.domainId}\n`,
  );
} catch (error) {
  await Promise.allSettled([
    firstTab.screenshot({ path: "/tmp/auki-camera-mesh-first-tab.png", fullPage: true }),
    secondTab.screenshot({ path: "/tmp/auki-camera-mesh-second-tab.png", fullPage: true }),
  ]);
  const diagnostics = await Promise.all([
    firstTab.locator("#events").innerText().catch(() => "unavailable"),
    secondTab.locator("#events").innerText().catch(() => "unavailable"),
  ]);
  throw new Error(
    `${error instanceof Error ? error.message : String(error)}\nfirst-tab events:\n${diagnostics[0]}\nsecond-tab events:\n${diagnostics[1]}\nconsole errors:\n${consoleErrors.join("\n")}`,
  );
} finally {
  await Promise.allSettled([stopIfRunning(firstTab), stopIfRunning(secondTab)]);
  await browser.close();
}

async function runCameraFlow(publisher, viewer, { label, capture, target }) {
  const publisherDomain = await loginAndStart(publisher, "publisher", label);
  const viewerDomain = await loginAndStart(viewer, "viewer", label);
  if (publisherDomain !== viewerDomain) {
    throw new Error(`tabs selected different Domains: ${publisherDomain} and ${viewerDomain}`);
  }

  const viewerCard = JSON.parse(await viewer.locator("#local-card").innerText());
  await publisher.locator("#capture-source").selectOption(capture);
  await publisher.locator("#publish-button").click();
  await waitForText(publisher, "#events", "Stream endpoint mounted", 30_000);
  if (capture === "webcam") {
    await waitForText(publisher, "#events", "Webcam permission granted", 30_000);
  }
  const publisherCard = JSON.parse(await publisher.locator("#local-card").innerText());

  if (target === "discovery") {
    await waitForCandidate(viewer, publisherCard.peerId, 60_000);
  } else {
    await viewer.locator(".manual-target > summary").click();
    await viewer.locator("#manual-card").fill(JSON.stringify(publisherCard));
    await viewer.locator("#add-card-button").click();
    await waitForText(viewer, "#events", "from a peer card", 10_000);
  }
  await viewer.locator("#candidate").selectOption(publisherCard.peerId);
  await viewer.locator("#connect-button").click();
  await waitForText(viewer, "#events", "approval requested", 30_000);

  const approval = publisher.locator("#pending-list li", { hasText: viewerCard.peerId }).locator("button", { hasText: "Allow" });
  await approval.waitFor({ state: "visible", timeout: 30_000 });
  await approval.click();

  await viewer.locator("#connect-button").click();
  await viewer.locator("#remote-frame").waitFor({ state: "visible", timeout: 45_000 });
  await waitFor(async () => {
    const loaded = await viewer.locator("#remote-frame").evaluate((image) => image.naturalWidth > 0);
    return loaded && await receivedFrames(viewer) >= 2;
  }, 30_000, "at least two decoded camera frames");

  const inspector = await viewer.locator("#inspector-details").innerText();
  if (!inspector.includes('"verified": true') || !inspector.includes(`Smoke ${label} publisher`)) {
    throw new Error("Protocol Inspector did not expose verified publisher metadata");
  }

  await viewer.locator("#pause-button").click();
  await waitForText(publisher, "#events", "Camera paused by an approved viewer", 30_000);
  await new Promise((resolve) => setTimeout(resolve, 1_000));
  const pausedAt = await receivedFrames(viewer);
  await new Promise((resolve) => setTimeout(resolve, 1_000));
  const pausedAfter = await receivedFrames(viewer);
  if (pausedAfter !== pausedAt) {
    throw new Error(`Message pause did not stop frames: ${pausedAt} -> ${pausedAfter}`);
  }

  await viewer.locator("#resume-button").click();
  await waitForText(publisher, "#events", "Camera resumed by an approved viewer", 30_000);
  await waitFor(
    async () => await receivedFrames(viewer) > pausedAfter,
    30_000,
    "camera frames after Message resume",
  );

  await viewer.locator("#snapshot-button").click();
  await viewer.locator("#snapshot-image").waitFor({ state: "visible", timeout: 30_000 });
  await waitFor(async () => {
    const loaded = await viewer.locator("#snapshot-image").evaluate((image) => image.naturalWidth > 0);
    const status = await viewer.locator("#snapshot-status").innerText();
    return loaded && /\b[0-9a-f]{64}\b/.test(status);
  }, 30_000, "SHA-256-verified Blob snapshot");
  await waitForText(viewer, "#events", "fetched and SHA-256 verified", 30_000);

  await publisher.locator("#stop-publish-button").click();
  await waitForText(publisher, "#events", "Camera publication stopped", 30_000);
  await viewer.locator("#remote-frame").waitFor({ state: "hidden", timeout: 30_000 });
  await waitForText(viewer, "#remote-empty", "Camera", 30_000);

  await publisher.locator("#publish-button").click();
  await waitFor(
    async () => !(await publisher.locator("#stop-publish-button").isDisabled()),
    30_000,
    "camera republish",
  );
  await viewer.locator("#connect-button").click();
  await approval.waitFor({ state: "visible", timeout: 30_000 });
  await waitFor(
    async () => !(await viewer.locator("#connect-button").isDisabled()),
    30_000,
    "unapproved reconnect rejection",
  );

  await viewer.locator("#stop-peer-button").click();
  await publisher.locator("#stop-peer-button").click();
  await Promise.all([
    waitForText(viewer, "#events", "relay booking released", 30_000),
    waitForText(publisher, "#events", "relay booking released", 30_000),
    waitForText(viewer, "#local-card", "Peer stopped", 30_000),
    waitForText(publisher, "#local-card", "Peer stopped", 30_000),
  ]);

  return {
    publisherPeerId: publisherCard.peerId,
    viewerPeerId: viewerCard.peerId,
    domainId: publisherDomain,
  };
}

async function loginAndStart(page, role, label) {
  await page.goto(url, { waitUntil: "networkidle" });
  await page.locator("#login-button").waitFor({ state: "visible" });
  await page.locator("#email").fill(email);
  await page.locator("#password").fill(password);
  await page.locator("#login-button").click();
  await page.locator("#peer-config").waitFor({ state: "visible", timeout: 30_000 });

  const domainIds = await page.locator("#domain option").evaluateAll(
    (options) => options.map((option) => option.value),
  );
  const selectedDomain = requestedDomain ?? domainIds[0];
  if (!selectedDomain || !domainIds.includes(selectedDomain)) {
    throw new Error(`requested Domain is unavailable for ${role}`);
  }
  await page.locator("#domain").selectOption(selectedDomain);
  await page.locator("#role").selectOption(role);
  await page.locator("#display-name").fill(`Smoke ${label} ${role}`);
  await page.locator("#start-peer-button").click();
  await page.locator("#runtime-section").waitFor({ state: "visible", timeout: 60_000 });
  return selectedDomain;
}

async function waitForCandidate(page, peerId, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    await page.locator("#discover-button").click();
    await waitFor(async () => !(await page.locator("#discover-button").isDisabled()), 15_000, "DDS refresh");
    const values = await page.locator("#candidate option").evaluateAll(
      (options) => options.map((option) => option.value),
    );
    if (values.includes(peerId)) return;
    await new Promise((resolve) => setTimeout(resolve, 1_000));
  }
  throw new Error(`DDS did not return camera publisher ${peerId}`);
}

async function receivedFrames(page) {
  const metrics = await page.locator("#metrics").innerText();
  return Number.parseInt(metrics.match(/^(\d+) received/)?.[1] ?? "0", 10);
}

async function stopIfRunning(page) {
  if (page.isClosed()) return;
  const runtime = page.locator("#runtime-section");
  if (await runtime.count() === 0 || !(await runtime.isVisible())) return;
  const local = await page.locator("#local-card").innerText();
  if (local.includes("Peer stopped")) return;
  await page.locator("#stop-peer-button").click({ timeout: 5_000 });
  await waitForText(page, "#local-card", "Peer stopped", 20_000);
}

async function waitForText(page, selector, text, timeoutMs) {
  await waitFor(
    async () => (await page.locator(selector).innerText()).toLowerCase().includes(text.toLowerCase()),
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
    await new Promise((resolve) => setTimeout(resolve, 200));
  }
  throw new Error(`timed out waiting for ${description}${lastError ? `: ${lastError}` : ""}`);
}
