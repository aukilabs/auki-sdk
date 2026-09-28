// Local UI checks only. No login, relay booking, or app offline mode.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdir } from "node:fs/promises";
import { once } from "node:events";
import { chromium } from "playwright";
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
