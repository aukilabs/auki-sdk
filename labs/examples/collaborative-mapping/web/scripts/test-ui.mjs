// Local UI checks only. No login, relay booking, or app offline mode.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdir } from "node:fs/promises";
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
let output = "";
server.stderr.on("data", (data) => {
  output += data;
});
let browser;
try {
  let ready = false;
  for (let i = 0; i < 100; i++) {
    if (server.exitCode !== null) throw Error(output);
    try {
      ready = (await fetch(`http://127.0.0.1:${port}`)).ok;
    } catch {
      /* Server starting. */
    }
    if (ready) break;
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  assert.ok(ready, "Vite did not start");
  browser = await chromium.launch({ channel: "chrome", headless: true });
  const page = await browser.newPage({
    viewport: { width: 1440, height: 1100 },
    deviceScaleFactor: 1,
  });
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.route("**/*", (route) => {
    const url = new URL(route.request().url());
    return url.hostname === "127.0.0.1" ? route.continue() : route.abort();
  });
  await page.goto(`http://127.0.0.1:${port}`);
  await page
    .getByText("Browser runtime ready", { exact: true })
    .waitFor({ timeout: 60000 });
  assert.equal(await page.locator("#login-button").isEnabled(), true);
  assert.equal(await page.locator("#portal-name").isDisabled(), true);
  assert.equal(await page.locator("#running").isVisible(), false);
  await page.selectOption("#environment", "custom");
  assert.equal(await page.locator("#api").isVisible(), true);
  assert.equal(await page.locator("#api").getAttribute("required"), "");
  await page.selectOption("#environment", "dev");
  await mkdir("test-results", { recursive: true });
  await page.waitForTimeout(200);
  assert.equal(
    await page
      .locator("#login-button")
      .evaluate((el) => getComputedStyle(el).backgroundColor),
    "rgb(121, 96, 205)",
  );
  await page.screenshot({
    path: "test-results/initial-desktop.png",
    fullPage: true,
  });
  // Test the shared renderer in isolation. No replacement WASM API or simulated transport.
  const result = await page.evaluate(async () => {
    const { Grid } = await import("/src/grid.ts");
    const root = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    root.id = "test-grid";
    root.style.width = "600px";
    root.style.height = "600px";
    document.body.append(root);
    let clicked;
    const grid = new Grid(root, (x, y) => {
      clicked = [x, y];
    });
    grid.render(
      [
        { id: "1", name: "apple", x: 1, y: 2, contributors: ["a"] },
        { id: "2", name: "banana", x: -6, y: -15, contributors: ["b"] },
        { id: "3", name: "bridge", x: 0, y: 0, contributors: ["a", "b"] },
      ],
      ["b", "a"],
    );
    const point = new DOMPoint(3, -4).matrixTransform(root.getScreenCTM());
    root.dispatchEvent(
      new MouseEvent("click", { clientX: point.x, clientY: point.y }),
    );
    const result = {
      count: root.querySelectorAll("[data-portal]").length,
      clicked,
      bridge: root
        .querySelector('[data-portal="bridge"] circle:last-of-type')
        ?.getAttribute("fill"),
      banana: root
        .querySelector('[data-portal="banana"]')
        ?.getAttribute("transform"),
    };
    root.remove();
    return result;
  });
  assert.deepEqual(result, {
    count: 3,
    clicked: [3, 4],
    bridge: "#52a897",
    banana: "translate(-6 15)",
  });
  const views = await page.evaluate(async () => {
    const { MapViews } = await import("/src/map-views.ts");
    const { Grid } = await import("/src/grid.ts");
    const removed = [];
    const maps = new MapViews(new Grid(document.getElementById("grid")), name => removed.push(name));
    const portal = (name, x, y, peer) => ({ id: name, name, x, y, contributors: [peer] });
    const view = {
      state: "aligned", reason: null, local_peer: "b", remote_peer: "a",
      local_frame: "frame-b", remote_frame: "frame-a", display_frame: "frame-a",
      conflicts: [], local_portals: [portal("bridge", 10, 20, "b")],
      remote_portals: [portal("bridge", 0, 0, "a"), portal("partner-only", 2, 3, "a")],
      portals: [portal("bridge", 0, 0, "a"), portal("partner-only", 2, 3, "a")],
    };
    maps.render(view);
    const aligned = {
      local: document.querySelector('#grid [data-portal="bridge"]').getAttribute("transform"),
      remote: document.querySelector('#remote-grid [data-portal="bridge"]').getAttribute("transform"),
      combined: document.querySelectorAll('#combined-grid [data-portal]').length,
      buttons: document.querySelectorAll('#portal-list button').length,
    };
    view.state = "conflict";
    view.local_portals.push(portal("bad", 4, 5, "b"));
    view.remote_portals.push(portal("bad", 9, 9, "a"));
    view.conflicts = [{ name: "bad", disagrees_with: ["bridge"] }, { name: "bridge", disagrees_with: ["bad"] }];
    maps.render(view);
    const conflict = {
      combinedHidden: document.getElementById("combined-grid").hasAttribute("hidden"),
      combinedCount: document.querySelectorAll('#combined-grid [data-portal]').length,
      highlighted: document.querySelectorAll('[data-conflict="true"]').length,
      evidence: document.getElementById("portal-list").textContent.includes("Incompatible with: bridge"),
    };
    document.querySelector('[aria-label="Remove bad from my map"]').click();
    return { aligned, conflict, removed };
  });
  assert.deepEqual(views, {
    aligned: { local: "translate(10 -20)", remote: "translate(0 0)", combined: 2, buttons: 1 },
    conflict: { combinedHidden: true, combinedCount: 0, highlighted: 4, evidence: true },
    removed: ["bad"],
  });
  await page.screenshot({ path: "test-results/conflict-desktop.png", fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  assert.equal(
    await page.evaluate(
      () => document.documentElement.scrollWidth > innerWidth,
    ),
    false,
  );
  await page.screenshot({
    path: "test-results/initial-mobile.png",
    fullPage: true,
  });
  await page.locator("#portal-panel").scrollIntoViewIfNeeded();
  await page.screenshot({ path: "test-results/conflict-mobile.png" });
  assert.equal(await page.locator("#remote-card").count(), 0);
  assert.equal(await page.locator("#local-card").count(), 0);
  await checkDiscovery(page);
  assert.deepEqual(errors, []);
  console.log(
    "Passed: real WASM load, relay-only controls, environment form, grid geometry/colors, click conversion, mobile overflow.",
  );
} finally {
  await browser?.close();
  if (server.exitCode === null) {
    server.kill("SIGTERM");
    await once(server, "exit");
  }
}
