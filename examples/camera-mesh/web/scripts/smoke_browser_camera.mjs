import { chromium } from "playwright";

const url = process.argv[2] ?? process.env.CAMERA_MESH_URL ?? "http://127.0.0.1:5173/";
const email = process.env.AUKI_EMAIL;
const password = process.env.AUKI_PASSWORD;
const requestedDomain = process.env.AUKI_DOMAIN_ID;

if (!email || !password) {
  throw new Error("AUKI_EMAIL and AUKI_PASSWORD are required");
}

const browser = await chromium.launch({ headless: true });
const context = await browser.newContext();
const publisher = await context.newPage();
const viewer = await context.newPage();
const pageErrors = [];

for (const [name, page] of [["publisher", publisher], ["viewer", viewer]]) {
  page.on("pageerror", (error) => pageErrors.push(`${name}: ${error.message}`));
}

try {
  const publisherDomain = await loginAndStart(publisher, "publisher");
  const viewerDomain = await loginAndStart(viewer, "viewer");
  if (publisherDomain !== viewerDomain) {
    throw new Error(`tabs selected different Domains: ${publisherDomain} and ${viewerDomain}`);
  }

  const publisherCard = JSON.parse(await publisher.locator("#local-card").innerText());
  const viewerCard = JSON.parse(await viewer.locator("#local-card").innerText());
  await publisher.locator("#capture-source").selectOption("synthetic");
  await publisher.locator("#publish-button").click();
  await waitForText(publisher, "#events", "Stream endpoint mounted", 30_000);

  await waitForCandidate(viewer, publisherCard.peerId, 60_000);
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
    const metrics = await viewer.locator("#metrics").innerText();
    const received = Number.parseInt(metrics.match(/^(\d+) received/)?.[1] ?? "0", 10);
    return loaded && received >= 2;
  }, 30_000, "at least two decoded camera frames");

  if (pageErrors.length) throw new Error(`browser page errors: ${pageErrors.join("; ")}`);

  await viewer.locator("#stop-peer-button").click();
  await publisher.locator("#stop-peer-button").click();
  await Promise.all([
    waitForText(viewer, "#local-card", "Peer stopped", 30_000),
    waitForText(publisher, "#local-card", "Peer stopped", 30_000),
  ]);

  process.stdout.write(
    `camera mesh smoke passed: ${publisherCard.peerId} -> browser viewer in Domain ${publisherDomain}\n`,
  );
} catch (error) {
  await Promise.allSettled([
    publisher.screenshot({ path: "/tmp/auki-camera-mesh-publisher.png", fullPage: true }),
    viewer.screenshot({ path: "/tmp/auki-camera-mesh-viewer.png", fullPage: true }),
  ]);
  throw error;
} finally {
  await browser.close();
}

async function loginAndStart(page, role) {
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
  await page.locator("#display-name").fill(`Smoke ${role}`);
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
