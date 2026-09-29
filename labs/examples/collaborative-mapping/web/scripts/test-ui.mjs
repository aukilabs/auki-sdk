// Local fixtures only: real WASM load, controller tests, then test-only UI port fixture.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdir, readFile } from "node:fs/promises";
import { once } from "node:events";
import { chromium } from "playwright";
import { checkDiscovery } from "./discovery-checks.mjs";
const port = 18146;
const server = spawn(
  process.execPath,
  [
    "node_modules/vite/bin/vite.js",
    "--host",
    "127.0.0.1",
    "--port",
    String(port),
    "--strictPort",
  ],
  { stdio: "pipe" },
);
let browser;
try {
  for (let i = 0; i < 100; i++) {
    try {
      if ((await fetch(`http://127.0.0.1:${port}`)).ok) break;
    } catch {}
    await new Promise((r) => setTimeout(r, 100));
  }
  browser = await chromium.launch({ channel: "chrome", headless: true });
  const page = await browser.newPage({
      viewport: { width: 1440, height: 960 },
    }),
    errors = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.route("**/*", (route) =>
    new URL(route.request().url()).hostname === "127.0.0.1"
      ? route.continue()
      : route.abort(),
  );
  await page.goto(`http://127.0.0.1:${port}`);
  await page
    .getByText("Browser runtime ready", { exact: true })
    .waitFor({ timeout: 60000 });
  assert.equal(await page.locator("#add-pin").isDisabled(), true);
  await checkDiscovery(page);
  // Fixture replaces only the WASM-facing ports on this test page. Production has no offline path.
  const fixture = await readFile(
    new URL("./ui-fixture.mjs", import.meta.url),
    "utf8",
  );
  await page.route("**/pkg-web/auki_collaborative_mapping_web.js", (r) =>
    r.fulfill({ contentType: "application/javascript", body: fixture }),
  );
  await page.reload();
  await page.fill("#email", "test@example.test");
  await page.fill("#password", "fixture-only");
  await page.click("#login-button");
  await page.click("#start-button");
  await page.getByText("1 peer", { exact: true }).waitFor();
  assert.equal(await page.locator("#add-pin").isEnabled(), true);
  const grid = page.locator("#grid"),
    box = await grid.boundingBox();
  const before = await grid.getAttribute("viewBox");
  await page.mouse.move(box.x + box.width * 0.5, box.y + box.height * 0.5);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width * 0.65, box.y + box.height * 0.6, {
    steps: 8,
  });
  await page.mouse.up();
  assert.notEqual(await grid.getAttribute("viewBox"), before);
  assert.equal(await page.locator("#pin-dialog").isVisible(), false);
  await grid.dblclick({
    position: { x: box.width * 0.5, y: box.height * 0.5 },
  });
  await page.locator("#pin-dialog").waitFor({ state: "visible" });
  await page.fill("#portal-name", "bridge");
  await page.click("#save-pin");
  assert.equal(await grid.locator('[data-portal="bridge"]').count(), 1);
  const savedCamera = await grid.getAttribute("viewBox");
  // Add aligned and unaligned peers through the same renderer used by the app.
  await page.evaluate(async () => {
    const d = window.fixture.data;
    const portal = (name, x, y, peer) => ({
      id: name,
      name,
      x,
      y,
      contributors: [peer],
    });
    d.layers.push({
      peer: "peer-b",
      frame: "frame-b",
      sequence: 3,
      state: "aligned",
      portals: [portal("cafe", 11, 22, "peer-b")],
      aligned_portals: [portal("cafe", 1, 2, "peer-b")],
      to_display: {
        from_frame_id: "frame-b",
        to_frame_id: "local-frame",
        translation: [-10, -20, 0],
      },
    });
    d.layers.push({
      peer: "peer-c",
      frame: "frame-c",
      sequence: 1,
      state: "separate",
      portals: [portal("park", 4, 5, "peer-c")],
      aligned_portals: null,
      to_display: null,
    });
    // Re-render via the existing app by performing a local edit after this injected snapshot.
  });
  await page.click("#add-pin");
  await page.fill("#portal-name", "home");
  await page.fill("#x", "6");
  await page.fill("#y", "-4");
  await page.click("#save-pin");
  assert.equal(await grid.getAttribute("viewBox"), savedCamera);
  assert.equal(await grid.locator('[data-portal="cafe"]').count(), 1);
  assert.equal(await grid.locator('[data-portal="park"]').count(), 0);
  const b = page.locator(".layer-row").filter({ hasText: "Peer peer-b" });
  await b.locator('input[type="checkbox"]').uncheck();
  assert.equal(await grid.locator('[data-portal="cafe"]').count(), 0);
  await b.locator('input[type="checkbox"]').check();
  await page.selectOption("#coordinate-frame", "peer-c");
  assert.equal(await page.locator("#add-pin").isDisabled(), true);
  assert.equal(await grid.locator('[data-portal="park"]').count(), 1);
  assert.equal(await grid.locator('[data-portal="bridge"]').count(), 0);
  await page.selectOption("#coordinate-frame", "combined");
  await page.click("#zoom-in");
  assert.notEqual(await grid.getAttribute("viewBox"), savedCamera);
  await page.click("#fit");
  await mkdir("test-results", { recursive: true });
  await page.screenshot({ path: "test-results/layers-desktop.png" });
  async function dragPin(name, { command = true, cancel = false } = {}) {
    const marker = grid.locator(`[data-portal="${name}"] path`);
    const bounds = await marker.boundingBox();
    const scale = await grid.evaluate((el) => ({
      x: el.getScreenCTM().a,
      y: el.getScreenCTM().d,
    }));
    if (command) await page.keyboard.down("Meta");
    await page.mouse.move(
      bounds.x + bounds.width / 2,
      bounds.y + bounds.height / 2,
    );
    await page.mouse.down();
    await page.mouse.move(
      bounds.x + bounds.width / 2 + scale.x * 2,
      bounds.y + bounds.height / 2 - scale.y * 3,
      { steps: 8 },
    );
    if (cancel) await page.keyboard.press("Escape");
    await page.mouse.up();
    if (command) await page.keyboard.up("Meta");
  }
  const homeBefore = await page.evaluate(() =>
    window.fixture.local.portals.find((p) => p.name === "home"),
  );
  const cameraBeforeMove = await grid.getAttribute("viewBox");
  await dragPin("home");
  const moved = await page.evaluate(() =>
    window.fixture.local.portals.find((p) => p.name === "home"),
  );
  assert.equal(moved.x, homeBefore.x + 2);
  assert.equal(moved.y, homeBefore.y + 3);
  assert.equal(await grid.getAttribute("viewBox"), cameraBeforeMove);
  assert.equal(await page.locator("#pin-dialog").isVisible(), false);
  await dragPin("home", { cancel: true });
  assert.deepEqual(
    await page.evaluate(() =>
      window.fixture.local.portals.find((p) => p.name === "home"),
    ),
    moved,
  );
  await dragPin("cafe");
  assert.equal(await grid.getAttribute("viewBox"), cameraBeforeMove);
  assert.equal(await page.locator("#pin-dialog").isVisible(), false);
  assert.equal(
    await page.evaluate(() =>
      window.fixture.local.portals.some((p) => p.name === "cafe"),
    ),
    false,
  );
  await dragPin("home", { command: false });
  assert.notEqual(await grid.getAttribute("viewBox"), cameraBeforeMove);
  assert.deepEqual(
    await page.evaluate(() =>
      window.fixture.local.portals.find((p) => p.name === "home"),
    ),
    moved,
  );
  await page.click("#fit");
  // Inspect and remove an owned portal through the actual map click / pointer capture path.
  await grid.locator('[data-portal="bridge"] path').click();
  await page.locator("#pin-dialog").waitFor({ state: "visible" });
  assert.equal(await page.locator("#remove-pin").isVisible(), true);
  await page.click("#remove-pin");
  assert.equal(await grid.locator('[data-portal="bridge"]').count(), 0);
  await page.click("#add-pin");
  await page.fill("#portal-name", "canceled");
  await page.keyboard.press("Escape");
  assert.equal(await grid.locator('[data-portal="canceled"]').count(), 0);
  await page.setViewportSize({ width: 390, height: 844 });
  assert.equal(
    await page.evaluate(
      () => document.documentElement.scrollWidth > innerWidth,
    ),
    false,
  );
  await page.screenshot({ path: "test-results/layers-mobile.png" });
  await page.click("#stop-button");
  assert.equal(await page.locator("#add-pin").isDisabled(), true);
  assert.equal(await grid.locator("[data-portal]").count(), 0);
  assert.deepEqual(errors, []);
  console.log(
    "Passed: real WASM load; solo session; drag/zoom; named drop; cancel; layer toggles; frame isolation; ownership; removal; camera persistence; mobile layout; stop cleanup.",
  );
} finally {
  await browser?.close();
  if (server.exitCode === null) {
    server.kill("SIGTERM");
    await once(server, "exit");
  }
}
