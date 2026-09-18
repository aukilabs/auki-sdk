// Loopback provider fixture + actual rebuilt WASM. No SDK method replacements.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { chromium } from 'playwright';
import { startJobsFixture, CONFIG, INPUT } from './jobs-fixture.mjs';
import { DOMAIN, OTHER } from './fixture.mjs';
import { go, back, selectDomain } from './guided-ui.mjs';
const fixture = await startJobsFixture(), state = fixture.model.state;
const vite = spawn(process.execPath, ['node_modules/vite/bin/vite.js', '--config', 'tests/fleet-vite.config.mjs', '--host', '127.0.0.1', '--port', '18117', '--strictPort'], { stdio: 'pipe' });
let browser;
try {
  let ready = false;
  for (let i = 0; i < 100; i++) { try { ready = (await fetch('http://127.0.0.1:18117')).ok; } catch {} if (ready) break; await new Promise(r => setTimeout(r, 100)); }
  assert.ok(ready);
  browser = await chromium.launch({ headless: true, args: ['--no-sandbox', '--disable-dev-shm-usage'] });
  const context = await browser.newContext({ serviceWorkers: 'block', viewport: { width: 390, height: 844 } }), forbidden = [], errors = [], resources = [];
  await context.route('**/*', route => { const url = new URL(route.request().url()); if (![fixture.base, fixture.dataBase, 'http://127.0.0.1:18117'].includes(url.origin)) { forbidden.push(url.origin); return route.abort(); } return route.continue(); });
  const page = await context.newPage(); page.setDefaultTimeout(8000);
  page.on('pageerror', e => errors.push(e.message)); page.on('response', r => resources.push(r.url()));
  await page.goto('http://127.0.0.1:18117'); await go(page, 'settings');
  for (const id of ['api', 'dds']) await page.locator(`#${id}`).fill(fixture.base);
  await page.locator('#dms').fill(fixture.base + '/v1/'); await back(page);
  await page.locator('#email').fill('fleet@example.test'); await page.locator('#password').fill('synthetic-password'); await page.locator('#signin').click();
  await page.waitForFunction(() => document.querySelector('#domains')?.textContent.includes('Synthetic lab')).catch(async error => { console.error('Login status:', await page.locator('#session-status').textContent(), 'Page errors:', errors, 'Routes:', fixture.state.requests); throw error; });
  await selectDomain(page, DOMAIN); await page.locator('[data-view="jobs"]').click();
  const discover = async () => { if (await page.locator('#jobs-back').isVisible()) await page.locator('#jobs-back').click(); await page.waitForFunction(() => document.querySelector('#jobs-configure')?.disabled === false); await page.locator('#jobs-configure').click(); await page.locator('#fleet-refresh').click(); };
  const choose = async () => { await page.locator('[data-jobs-screen="dashboard"]').waitFor(); await page.locator('#jobs-new').click(); await page.locator('[data-jobs-screen="choose"]').waitFor(); };
  const settled = async () => page.waitForFunction(() => document.querySelector('#fleet-refresh')?.disabled === false);
  await discover(); await choose();
  // Initial chooser geometry: no scrollIntoView, trial click or other auto-scroll.
  await page.evaluate(() => document.fonts.ready);
  for (const selector of ['#jobs-search', '#jobs-estimate']) {
    const geometry = await page.locator(selector).evaluate(el => {
      const box = el.getBoundingClientRect(), pane = el.closest('.view').getBoundingClientRect();
      return { top: box.top, bottom: box.bottom, left: box.left, right: box.right, paneTop: pane.top, paneBottom: pane.bottom, width: innerWidth, height: innerHeight, scroll: el.closest('.view').scrollTop };
    });
    assert.equal(geometry.scroll, 0, 'initial chooser has not scrolled');
    assert.ok(geometry.top >= geometry.paneTop && geometry.bottom <= Math.min(geometry.height, geometry.paneBottom) && geometry.left >= 0 && geometry.right <= geometry.width, `${selector} fully visible initially: ${JSON.stringify(geometry)}`);
  }
  assert.equal(await page.locator('#jobs-installation').count(), 0);
  assert.match(await page.locator('#jobs-workers').textContent(), /Robot:/);
  assert.match(await page.locator('#jobs-workers').textContent(), /Compute:/);
  assert.ok(resources.some(url => /\.wasm(?:\?|$)/.test(url)));
  assert.ok(fixture.state.requests.some(r => r.includes('/robots')));
  assert.ok(fixture.state.requests.some(r => r.includes('/api/v1/nodes?')));
  state.fleetMultiple = true; await discover(); await settled();
  assert.equal(await page.locator('[data-fleet-installation]').count(), 2);
  assert.equal(await page.locator('#jobs-estimate').count(), 0);
  await page.locator(`[data-fleet-installation="${CONFIG.installationId}"]`).click(); await choose(); state.fleetMultiple = false;
  state.fleetDuplicate = true; await discover(); await choose();
  assert.equal(await page.locator('#jobs-role option[value="compute"]').evaluate(el => el.disabled), true, await page.locator('#view-jobs').textContent());
  assert.equal(await page.locator('#jobs-role').inputValue(), 'robot'); state.fleetDuplicate = false;
  state.fleetMissing = 'robot'; await discover(); await choose();
  assert.equal(await page.locator('#jobs-role option[value="robot"]').evaluate(el => el.disabled), true); state.fleetMissing = undefined;
  for (const mode of ['fleetBusyDenied', 'fleetOffline', 'fleetUnknown']) {
    state[mode] = true; await discover(); await choose();
    assert.match(await page.locator('#jobs-workers').textContent(), mode === 'fleetOffline' ? /offline/ : /unknown/); state[mode] = false;
  }
  for (const role of ['compute', 'robot']) {
    state.fleetCrossKind = role; await discover(); await choose();
    assert.equal(await page.locator(`#jobs-role option[value="${role}"]`).evaluate(el => el.disabled), true);
    state.fleetCrossKind = undefined;
  }
  for (const mode of ['empty', 'failed', 'ambiguous', 'denied']) {
    await discover(); await choose(); // Start each case with a usable configuration.
    if (mode === 'empty') state.fleetEmpty = true;
    if (mode === 'failed') state.fleetWrongDomain = true;
    if (mode === 'ambiguous') state.fleetCrossKind = 'both';
    if (mode === 'denied') state.fleetDenied = 'robots';
    await discover(); await settled();
    if (mode === 'empty') assert.match(await page.locator('#fleet-status').innerText(), /No demo installations found/);
    if (mode === 'failed') assert.match(await page.locator('#fleet-status').innerText(), /failed/);
    if (mode === 'denied') { await page.locator('#view-jobs summary').click(); assert.match(await page.locator('#view-jobs').innerText(), /denied/); }
    await page.locator('#jobs-back').click(); await choose();
    assert.equal(await page.locator('#jobs-estimate').isDisabled(), true, `${mode}: Back must not restore stale submission eligibility`);
    for (const role of ['compute', 'robot']) assert.equal(await page.locator(`#jobs-role option[value="${role}"]`).evaluate(el => el.disabled), true);
    assert.equal(await page.locator('#jobs-history').isDisabled(), false, 'dashboard history remains accessible');
    state.fleetEmpty = false; state.fleetWrongDomain = false; state.fleetCrossKind = undefined; state.fleetDenied = undefined;
  }
  // Dedicated Fleet screen uses the same actual WASM and local providers.
  state.fleetBroad = true;
  await page.locator('#jobs-back').click();
  await page.locator('[data-jobs-screen="dashboard"]:visible').waitFor();
  assert.ok(await page.locator('#jobs-view-fleet').isVisible());
  await page.locator('#jobs-view-fleet').click();
  const refreshFleet = async () => {
    await page.locator('#fleet-screen-refresh').click();
    await page.waitForFunction(() => document.querySelector('#fleet-screen-refresh')?.disabled === false);
  };
  await refreshFleet();
  assert.equal(await page.locator('[data-machine]').count(), 3);
  for (const [width, height] of [[320, 568], [640, 360], [320, 844], [390, 844], [1280, 844]]) {
    await page.setViewportSize({ width, height });
    assert.equal(await page.locator('[data-view]').count(), 5);
    assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
    for (const view of ['data', 'jobs', 'fleet', 'overview', 'networking']) {
      const box = await page.locator(`[data-view="${view}"]`).boundingBox();
      assert.ok(box && box.width >= 24 && box.height >= 24 && box.x >= 0 && box.x + box.width <= width + 1 && box.y >= 0 && box.y + box.height <= height + 1, `${view} fully visible at ${width}x${height}: ${JSON.stringify(box)}`);
    }
  }
  await page.locator('[data-fleet-mode="public"]').click();
  assert.equal(await page.locator('[data-machine]').count(), 1);
  await page.locator('[data-machine]').click();
  assert.match(await page.locator('#view-fleet').innerText(), /Visible compute candidate/);
  assert.match(await page.locator('#view-fleet').innerText(), /vendor\/arbitrary\/v9/);
  await page.locator('#view-fleet summary').click();
  assert.match(await page.locator('#view-fleet').innerText(), /compute_pool/);
  await page.locator('#fleet-screen-back').click();
  assert.equal(await page.locator('[data-fleet-mode="public"]').getAttribute('aria-pressed'), 'true');
  await page.locator('[data-fleet-mode="dedicated"]').click();
  assert.equal(await page.locator('[data-machine]').count(), 2);
  await page.locator('[data-fleet-mode="all"]').click();
  state.fleetDenied = 'robots'; await refreshFleet();
  assert.match(await page.locator('#fleet-screen-status').innerText(), /Partial/);
  state.fleetDenied = undefined; state.fleetEmpty = true; await refreshFleet();
  assert.match(await page.locator('#view-fleet').innerText(), /No machines in this view/);
  state.fleetEmpty = false; state.fleetWrongDomain = true; await refreshFleet();
  assert.match(await page.locator('#fleet-screen-status').innerText(), /Partial|failed/);
  state.fleetWrongDomain = false; await refreshFleet();
  assert.equal(await page.locator('[data-machine]').count(), 3);
  state.fleetHold = true; await page.locator('#fleet-screen-refresh').click();
  await fixture.waitFor(() => fixture.state.exchanges.some(e => e.held && !e.finished && !e.aborted));
  const fleetHeld = fixture.state.exchanges.filter(e => e.held && !e.finished && !e.aborted);
  await page.getByRole('button', { name: '← Jobs', exact: true }).click();
  await fixture.waitFor(() => fleetHeld.every(e => e.aborted));
  fixture.release(); state.fleetHold = false; state.fleetBroad = false;
  assert.equal(state.submissions.length, 0); assert.equal(state.estimates.length, 0);
  // One explicit synthetic submission with an invalid success response retains
  // reconciliation across the actual global Fleet navigation and read controls.
  await discover(); await choose();
  await page.locator('[data-role="compute"]').click();
  await page.locator(`[data-record-id="${INPUT}"]`).click();
  await page.waitForFunction(() => { const text = document.querySelector('#jobs-input-preview')?.textContent; return text && !text.includes('Loading preview'); });
  await page.locator('#jobs-estimate').click();
  await page.locator('[data-jobs-screen="review"]:visible').waitFor();
  await page.waitForFunction(() => document.querySelector('#jobs-confirm')?.disabled === false);
  state.uncertain = true;
  await page.locator('#jobs-confirm').click();
  await page.locator('[data-jobs-screen="uncertain"]:visible').waitFor();
  state.uncertain = false;
  assert.equal(state.submissions.length, 1); assert.equal(state.estimates.length, 1);
  assert.equal(state.requests.filter(r => r.method === 'POST' && r.path === '/v1/jobs').length, 1, 'one submission request across Fleet navigation and cleanup');
  const reconciliation = state.submissions[0].body.label;
  assert.ok((await page.locator('#view-jobs').innerText()).includes(reconciliation));
  await page.locator('[data-view="fleet"]').click();
  await refreshFleet();
  for (const mode of ['public', 'dedicated', 'all']) await page.locator(`[data-fleet-mode="${mode}"]`).click();
  await page.locator('[data-view="jobs"]').click();
  await page.locator('[data-jobs-screen="uncertain"]:visible').waitFor();
  assert.ok((await page.locator('#view-jobs').innerText()).includes(reconciliation));
  assert.equal(await page.locator('#jobs-confirm').count(), 0);
  assert.equal(await page.locator('#jobs-new:visible').count(), 0);
  assert.ok(await page.locator('#jobs-history').isEnabled());
  assert.equal(state.submissions.length, 1); assert.equal(state.estimates.length, 1);
  assert.equal(state.requests.filter(r => r.method === 'POST' && r.path === '/v1/jobs').length, 1, 'one submission request across Fleet navigation and cleanup');

  // Fleet-owned held inventory must drain on Domain change and logout. Record
  // provider exchanges so new-Domain entry cannot silently read or reuse results.
  const inventoryExchanges = () => fixture.state.exchanges.filter(e => e.path.endsWith('/robots') || e.path === '/api/v1/nodes');
  for (const action of ['domain', 'logout']) {
    await page.locator('[data-view="fleet"]').click();
    await refreshFleet();
    assert.ok(await page.locator(`[data-machine="${CONFIG.robotId}"]`).count());
    state.fleetHold = true;
    const started = inventoryExchanges().length;
    await page.locator('#fleet-screen-refresh').click();
    await fixture.waitFor(() => inventoryExchanges().slice(started).some(e => e.held && !e.finished && !e.aborted));
    if (action === 'domain') await selectDomain(page, OTHER);
    else await page.locator('#logout').click();
    await fixture.waitFor(() => inventoryExchanges().slice(started).every(e => e.aborted || e.finished));
    const held = inventoryExchanges().slice(started).filter(e => e.held);
    assert.ok(held.length > 0);
    assert.ok(held.every(e => e.aborted), 'Fleet-owned held reads aborted');
    fixture.release();
    if (action === 'domain') {
      const beforeEntry = inventoryExchanges().length;
      await page.locator('[data-view="fleet"]').click();
      assert.equal(await page.locator('[data-machine]').count(), 0, 'new Domain has no stale Fleet machines');
      assert.equal(inventoryExchanges().length, beforeEntry, 'Fleet entry does not request inventory');
      await refreshFleet();
      const fresh = inventoryExchanges().slice(beforeEntry);
      assert.ok(fresh.some(e => e.path === `/api/v1/domains/${OTHER}/robots`));
      assert.ok(!fresh.some(e => e.path === `/api/v1/domains/${DOMAIN}/robots`), 'no stale Domain request');
      assert.equal(await page.locator(`[data-machine="${CONFIG.robotId}"]`).count(), 0, 'old Domain robot stays absent after refresh');
      await selectDomain(page, DOMAIN);
    } else {
      await page.waitForFunction(() => document.querySelector('#session-status')?.textContent === 'Signed out');
      assert.equal(await page.locator('[data-machine]').count(), 0, 'logout clears Fleet machines');
    }
  }
  // Preserve the original Jobs-discovery cancellation checks in a fresh session.
  await page.locator('#email').fill('fleet-after-logout@example.test');
  await page.locator('#password').fill('synthetic-password'); await page.locator('#signin').click();
  await page.waitForFunction(() => document.querySelector('#domains')?.textContent.includes('Synthetic lab'));
  await selectDomain(page, DOMAIN); await page.locator('[data-view="jobs"]').click();
  for (const action of ['domain', 'logout']) {
    await page.waitForFunction(() => document.querySelector('#jobs-configure')?.disabled === false);
    state.fleetHold = true; await discover();
    await fixture.waitFor(() => fixture.state.exchanges.some(e => e.held && !e.finished && !e.aborted && (e.path.endsWith('/robots') || e.path === '/api/v1/nodes')));
    const held = fixture.state.exchanges.filter(e => e.held && !e.finished && !e.aborted);
    if (action === 'domain') await selectDomain(page, OTHER); else await page.locator('#logout').click();
    await fixture.waitFor(() => held.every(e => e.aborted)); fixture.release();
    if (action === 'domain') { await page.locator('[data-view="jobs"]').click(); assert.equal(await page.locator('[data-fleet-installation]').count(), 0); await selectDomain(page, DOMAIN); await page.locator('[data-view="jobs"]').click(); }
    else await page.waitForFunction(() => document.querySelector('#session-status')?.textContent === 'Signed out');
  }
  assert.equal(state.submissions.length, 1); assert.equal(state.estimates.length, 1);
  assert.equal(state.requests.filter(r => r.method === 'POST' && r.path === '/v1/jobs').length, 1, 'one submission request across Fleet navigation and cleanup');
  assert.deepEqual(state.violations, []); assert.deepEqual(forbidden, []); assert.deepEqual(errors, []);
  console.log('PASS real-WASM Fleet discovery, roles, sources, wrong Domain, cancellation and logout; loopback only.');
} finally {
  await browser?.close();
  if (vite.exitCode === null && vite.signalCode === null) { const exited = once(vite, 'exit'); vite.kill('SIGTERM'); await exited; }
  await fixture.close();
}
