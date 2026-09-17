// Loopback provider fixture + actual rebuilt WASM. No SDK method replacements.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { chromium } from 'playwright';
import { startJobsFixture, CONFIG } from './jobs-fixture.mjs';
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
  const discover = async () => { await page.locator('#jobs-configure').click(); await page.locator('#fleet-refresh').click(); };
  const choose = async () => page.locator('[data-jobs-screen="choose"]').waitFor();
  const settled = async () => page.waitForFunction(() => document.querySelector('#fleet-refresh')?.disabled === false);
  await discover(); await choose();
  // Initial chooser geometry: no scrollIntoView, trial click or other auto-scroll.
  await page.evaluate(() => document.fonts.ready);
  for (const selector of ['#jobs-input', '#jobs-estimate']) {
    const geometry = await page.locator(selector).evaluate(el => {
      const box = el.getBoundingClientRect(), pane = el.closest('.view').getBoundingClientRect();
      return { top: box.top, bottom: box.bottom, left: box.left, right: box.right, paneTop: pane.top, paneBottom: pane.bottom, width: innerWidth, height: innerHeight, scroll: el.closest('.view').scrollTop };
    });
    assert.equal(geometry.scroll, 0, 'initial chooser has not scrolled');
    assert.ok(geometry.top >= geometry.paneTop && geometry.bottom <= Math.min(geometry.height, geometry.paneBottom) && geometry.left >= 0 && geometry.right <= geometry.width, `${selector} fully visible initially: ${JSON.stringify(geometry)}`);
  }
  assert.equal(await page.locator('#jobs-installation').count(), 0);
  assert.match(await page.locator('#view-jobs').innerText(), /assigned robot/);
  assert.match(await page.locator('#view-jobs').innerText(), /dedicated candidate/);
  assert.ok(resources.some(url => /\.wasm(?:\?|$)/.test(url)));
  assert.ok(fixture.state.requests.some(r => r.includes('/robots')));
  assert.ok(fixture.state.requests.some(r => r.includes('/api/v1/nodes?')));
  state.fleetMultiple = true; await discover(); await settled();
  assert.equal(await page.locator('[data-fleet-installation]').count(), 2);
  assert.equal(await page.locator('#jobs-estimate').count(), 0);
  await page.locator(`[data-fleet-installation="${CONFIG.installationId}"]`).click(); await choose(); state.fleetMultiple = false;
  state.fleetDuplicate = true; await discover(); await choose();
  assert.equal(await page.locator('#jobs-role option[value="compute"]').evaluate(el => el.disabled), true, await page.locator('#view-jobs pre').textContent());
  assert.equal(await page.locator('#jobs-role').inputValue(), 'robot'); state.fleetDuplicate = false;
  state.fleetMissing = 'robot'; await discover(); await choose();
  assert.equal(await page.locator('#jobs-role option[value="robot"]').evaluate(el => el.disabled), true); state.fleetMissing = undefined;
  for (const mode of ['fleetBusyDenied', 'fleetOffline', 'fleetUnknown']) {
    state[mode] = true; await discover(); await choose();
    assert.match(await page.locator('#view-jobs').innerText(), mode === 'fleetOffline' ? /offline/ : /unknown/); state[mode] = false;
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
    assert.equal(await page.locator('#jobs-history').isDisabled(), false, 'history remains accessible');
    state.fleetEmpty = false; state.fleetWrongDomain = false; state.fleetCrossKind = undefined; state.fleetDenied = undefined;
  }
  for (const action of ['domain', 'logout']) {
    state.fleetHold = true; await discover();
    await fixture.waitFor(() => fixture.state.exchanges.some(e => e.held && !e.finished && !e.aborted && (e.path.endsWith('/robots') || e.path === '/api/v1/nodes')));
    const held = fixture.state.exchanges.filter(e => e.held && !e.finished && !e.aborted);
    if (action === 'domain') await selectDomain(page, OTHER); else await page.locator('#logout').click();
    await fixture.waitFor(() => held.every(e => e.aborted)); fixture.release();
    if (action === 'domain') { await page.locator('[data-view="jobs"]').click(); assert.equal(await page.locator('[data-fleet-installation]').count(), 0); await selectDomain(page, DOMAIN); await page.locator('[data-view="jobs"]').click(); }
    else await page.waitForFunction(() => document.querySelector('#session-status')?.textContent === 'Signed out');
  }
  assert.equal(state.submissions.length, 0); assert.equal(state.estimates.length, 0);
  assert.deepEqual(state.violations, []); assert.deepEqual(forbidden, []); assert.deepEqual(errors, []);
  console.log('PASS real-WASM Fleet discovery, roles, sources, wrong Domain, cancellation and logout; loopback only.');
} finally {
  await browser?.close();
  if (vite.exitCode === null && vite.signalCode === null) { const exited = once(vite, 'exit'); vite.kill('SIGTERM'); await exited; }
  await fixture.close();
}
