// Parent-run only, after the real WASM SDK and app are built. This harness starts
// loopback synthetic HTTP providers + preview; it never builds or runs workers.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { mkdir, readFile } from 'node:fs/promises';
import { chromium } from 'playwright';
import { DOMAIN, OTHER, TEXT } from './fixture.mjs';
import { startJobsFixture, CONFIG, INPUT, COST, CURSOR, capability, assertJobSpec } from './jobs-fixture.mjs';
import { go, back, selectDomain, assertUsable, reveal, unexpectedConsoleError } from './guided-ui.mjs';
const fixture = await startJobsFixture(), state = fixture.model.state;
const vite = spawn(process.execPath, ['node_modules/vite/bin/vite.js', 'preview', '--host', '127.0.0.1', '--port', '18116', '--strictPort'], { stdio: 'pipe' });
let browser;
try {
  let ready = false;
  for (let i = 0; i < 100; i++) {
    try { ready = (await fetch('http://127.0.0.1:18116')).ok; } catch {}
    if (ready) break; await new Promise(resolve => setTimeout(resolve, 100));
  }
  assert.ok(ready, 'parent must build preview first');
  browser = await chromium.launch({ headless: true, args: ['--no-sandbox', '--disable-dev-shm-usage'] });
  const context = await browser.newContext({ acceptDownloads: true, serviceWorkers: 'block', viewport: { width: 1440, height: 900 } });
  const forbidden = [], errors = [], logs = [], resources = [];
  const allowed = new Set(['http://127.0.0.1:18116', fixture.base, fixture.dataBase]);
  await context.route('**/*', route => {
    const url = new URL(route.request().url());
    if (!allowed.has(url.origin)) { forbidden.push(url.origin + url.pathname); return route.abort(); }
    return route.continue();
  });
  const page = await context.newPage(); page.setDefaultTimeout(10000);
  page.on('pageerror', error => errors.push(error.message));
  page.on('response', response => resources.push(response.url()));
  page.on('console', message => { logs.push(message.text()); if (unexpectedConsoleError(message) && !/status of 400\b/.test(message.text())) errors.push(message.text()); });
  // Observe actual browser fetches, without fulfilling requests or replacing SDK methods.
  await page.addInitScript(() => {
    const fetch = window.fetch; window.__jobsPosts = [];
    window.fetch = function(input, init) {
      const url = new URL(input instanceof Request ? input.url : String(input), location.href);
      const method = init?.method ?? (input instanceof Request ? input.method : 'GET');
      if (method.toUpperCase() === 'POST' && url.pathname === '/v1/jobs') window.__jobsPosts.push(url.pathname);
      return fetch.apply(this, arguments);
    };
  });
  await page.goto('http://127.0.0.1:18116'); assert.equal(fixture.state.requests.length, 0);
  await go(page, 'settings');
  for (const id of ['api', 'dds']) await page.locator(`#${id}`).fill(fixture.base);
  await page.locator('#dms').fill(fixture.base + '/v1/'); await back(page);
  const login = async (email = 'jobs-a@example.test', domain = DOMAIN) => {
    await page.locator('#email').fill(email); await page.locator('#password').fill('synthetic-password'); await page.locator('#signin').click();
    await page.waitForFunction(() => document.querySelector('#domains')?.textContent.includes('Synthetic lab'));
    await selectDomain(page, domain);
    if (domain === DOMAIN) await page.waitForFunction(() => document.querySelector('#records')?.textContent.includes('Synthetic jobs input'));
  };
  const open = async () => { await page.locator('[data-view="jobs"]').click(); await page.locator('#view-jobs:visible').waitFor(); };
  const step = async name => { await page.locator(`[data-jobs-screen="${name}"]:visible`).waitFor(); assert.equal(await page.locator('[data-jobs-screen]:visible').count(), 1); };
  const configure = async () => {
    await open(); await page.locator('#jobs-configure').click(); await step('setup');
    await page.locator('#fleet-refresh').click(); await step('choose');
  };
  let confirmations = 0;
  const noRetry = async () => { await page.waitForTimeout(700); assert.equal(await page.evaluate(() => window.__jobsPosts.length), confirmations, 'exactly one actual SDK fetch per explicit confirmation'); };
  const prepare = async (role = 'compute', input = INPUT) => {
    await configure(); await page.locator('#jobs-role').selectOption(role); await page.locator('#jobs-input').fill(input);
    const before = state.submissions.length;
    await page.locator('#jobs-estimate').click(); await step('review');
    assert.equal(state.submissions.length, before, 'estimate never submits');
    assert.equal(await page.evaluate(() => window.__jobsPosts.length), confirmations);
    const text = await page.locator('#view-jobs').innerText();
    for (const value of [COST, DOMAIN, CONFIG[`${role}Id`], input, capability(role), `sdk-${CONFIG.installationId}-${role}-{taskId}`, role === 'robot' ? 'example.report.v1' : 'example.text.v1']) assert.ok(text.includes(value));
    assert.match(text, /synthetic test data/i); assertJobSpec(state.estimates.at(-1));
    assert.equal(state.estimates.at(-1).tasks[0].capability, capability(role));
  };
  const submit = async (double = false) => {
    confirmations++;
    // Two physical clicks are delivered to the same location; disabled/replaced
    // controls must suppress the second intent without force or DOM dispatch.
    if (double) await page.locator('#jobs-confirm').click({ clickCount: 2 });
    else await page.locator('#jobs-confirm').click();
  };
  await login(); assert.equal(state.requests.length, 0, 'no jobs request until user action');
  await open(); await step('setup'); assert.match(await page.locator('#view-jobs').innerText(), /activated|activation/i);
  assert.ok(resources.some(url => /\.wasm(?:\?|$)/.test(url)), 'actual WASM resource loaded');
  for (const role of ['compute', 'robot']) {
    await prepare(role); const estimated = structuredClone(state.estimates.at(-1)); await submit(true); await step('detail'); await noRetry();
    const submitted = state.submissions.at(-1); assert.deepEqual(submitted.body, estimated, 'exact reviewed spec submitted');
    await page.locator('[data-job-output]').waitFor();
    assert.equal(await page.locator('[data-job-output]').count(), 1);
    assert.ok(await page.locator('#jobs-new').isVisible());
    assert.match(await page.locator('#jobs-tasks').innerText(), /Phase|Recent events/);
    assert.match(await page.locator('#jobs-tasks').innerText(), /synthetic fixture result/);
    await page.locator('[data-job-output]').click(); await page.locator('#view-record:visible').waitFor();
    const outputName = fixture.records.find(record => record.id === submitted.output).name;
    await page.waitForFunction(name => document.querySelector('#record-name')?.textContent === name, outputName);
    await page.locator('#record-technical').click();
    assert.ok((await page.locator('#technical-content').innerText()).includes(submitted.output), 'output resolves through current Domain data and its public metadata screen');
    await back(page);
    await reveal(page, '#download'); const downloading = page.waitForEvent('download'); await page.locator('#download').click();
    const download = await downloading; assert.deepEqual(await readFile(await download.path()), fixture.contents.get(submitted.output));
    // The existing record action carries its UUID into the jobs chooser.
    await reveal(page, '#open-record-jobs'); await page.locator('#open-record-jobs').click(); await step('choose');
    assert.equal(await page.locator('#jobs-input').inputValue(), submitted.output);
  }
  // Edit invalidates the previous review and requires a new exact estimate.
  await prepare(); const count = state.estimates.length; await page.locator('#jobs-edit').click(); await step('choose');
  assert.equal(await page.locator('#jobs-confirm').count(), 0);
  await page.locator('#jobs-role').selectOption('robot'); await page.locator('#jobs-input').fill(TEXT); await page.locator('#jobs-estimate').click(); await step('review');
  assert.equal(state.estimates.length, count + 1); assert.equal(state.estimates.at(-1).tasks[0].meta.input_id, TEXT);
  assert.equal(state.estimates.at(-1).tasks[0].capability, capability('robot'));
  await page.locator('#jobs-configure').click(); await step('setup'); assert.equal(await page.locator('#jobs-confirm').count(), 0);
  // Layout and keyboard-reachable controls across desktop and narrow phones.
  const artifacts = new URL('../test-artifacts/', import.meta.url); await mkdir(artifacts, { recursive: true });
  for (const [width, height] of [[1440, 900], [390, 844], [320, 568]]) {
    await page.setViewportSize({ width, height }); await configure();
    await assertUsable(page, ['#jobs-role', '#jobs-input', '#jobs-estimate', '#jobs-back']);
    await page.screenshot({ path: new URL(`jobs-${width}x${height}.png`, artifacts).pathname, fullPage: true });
  }
  await page.setViewportSize({ width: 1440, height: 900 });
  for (const mode of ['mismatch', 'missingExecutor']) {
    state[mode] = true; await prepare('robot'); await submit(); await step('detail'); await noRetry();
    assert.equal(await page.locator('[data-job-output]').count(), 0); assert.match(await page.locator('#view-jobs').innerText(), /unverified|mismatch|withheld/i); state[mode] = false;
  }
  for (const mode of ['deny', 'noWorkers']) {
    await configure(); await page.locator('#jobs-input').fill(INPUT); state[mode] = true;
    const before = state.requests.length; await page.locator('#jobs-estimate').click();
    await fixture.waitFor(() => state.requests.length > before);
    await page.waitForFunction(() => /denied|permission|worker|eligible|available|fail|rejected/i.test(document.querySelector('#jobs-status')?.textContent ?? ''));
    assert.equal(await page.locator('#jobs-confirm:visible').count(), 0); await noRetry(); state[mode] = false;
  }
  state.running = true; await prepare(); await submit(); await step('detail');
  const running = state.submissions.at(-1); const cancellations = state.cancellations.length;
  page.once('dialog', dialog => dialog.dismiss()); await page.locator('#jobs-cancel').click(); assert.equal(state.cancellations.length, cancellations);
  page.once('dialog', dialog => dialog.accept()); await page.locator('#jobs-cancel').click();
  await fixture.waitFor(() => state.cancellations.length === cancellations + 1);
  await page.waitForFunction(() => document.querySelector('#jobs-status')?.textContent.includes('does not prove'));
  assert.equal(state.cancellations.length, cancellations + 1);
  assert.equal(fixture.model.jobs.get(running.id).tasks[0].status, 'running'); assert.equal(fixture.model.jobs.get(running.id).job.credit_released_at, null);
  assert.match(await page.locator('#view-jobs').innerText(), /does not prove|not.*stopp|still.*running/i); state.running = false;
  state.uncertain = true; await prepare(); await submit(); await step('uncertain'); await noRetry();
  assert.equal(await page.locator('#jobs-confirm').count(), 0); assert.match(await page.locator('#view-jobs').innerText(), /reconcile|history/i);
  const ambiguous = state.submissions.at(-1); assert.ok((await page.locator('#view-jobs').innerText()).includes(ambiguous.body.label)); state.uncertain = false;
  await selectDomain(page, OTHER); await open(); await step('setup');
  assert.ok(!(await page.locator('#view-jobs').innerText()).includes(ambiguous.body.label));
  await selectDomain(page, DOMAIN); await open(); await step('uncertain');
  assert.ok((await page.locator('#view-jobs').innerText()).includes(ambiguous.body.label));
  assert.equal(await page.locator('#jobs-confirm').count(), 0);
  assert.equal(await page.locator('#jobs-new:visible').count(), 0); await noRetry();
  const listed = state.lists.length; await page.locator('#jobs-history').click(); await step('history');
  await page.locator(`[data-job-id="${ambiguous.id}"]`).waitFor();
  assert.equal(state.lists.length, listed + 1); assert.match(await page.locator('#view-jobs').innerText(), /396/); assert.match(await page.locator('#view-jobs').innerText(), /incomplete|skip/i);
  assert.ok(await page.locator('[data-job-id]').count() > 0);
  await page.locator(`[data-job-id="${ambiguous.id}"]`).click(); await step('detail');
  await page.locator('[data-job-output]').waitFor();
  await page.locator('#jobs-history').click(); await step('history');
  await page.waitForFunction(() => document.querySelector('#jobs-next')?.disabled === false);
  const pages = state.lists.length; await page.locator('#jobs-next').click();
  await fixture.waitFor(() => state.lists.length === pages + 1);
  await page.waitForFunction(() => document.querySelector('#jobs-next')?.disabled);
  assert.equal(state.lists.length, pages + 1); assert.equal(new URLSearchParams(state.lists.at(-1)).get('cursor'), CURSOR);
  assert.match(await page.locator('#view-jobs').innerText(), /not proof|absent|skip/i);
  await page.locator('#jobs-new').click(); await step('choose');
  // A sent submit whose response is aborted retains recovery through A -> B -> A.
  await prepare(); state.hold = 'all'; await submit();
  await fixture.waitFor(() => fixture.state.exchanges.some(e => e.path === '/v1/jobs' && e.held && !e.finished && !e.aborted));
  const heldSubmit = fixture.state.exchanges.filter(e => e.path === '/v1/jobs' && e.held).at(-1);
  const sent = state.submissions.at(-1);
  await selectDomain(page, OTHER); await fixture.waitFor(() => heldSubmit.aborted); fixture.release();
  await open(); await step('setup');
  assert.ok(!(await page.locator('#view-jobs').innerText()).includes(sent.body.label));
  await selectDomain(page, DOMAIN); await open(); await step('uncertain');
  assert.ok((await page.locator('#view-jobs').innerText()).includes(sent.body.label)); await noRetry();
  // Explicit logout wipes unresolved recovery, even on returning to its Domain.
  await page.locator('#logout').click(); await page.waitForFunction(() => document.querySelector('#session-status')?.textContent === 'Signed out');
  await login('jobs-after-uncertain@example.test', OTHER); await open(); await step('setup');
  assert.ok(!(await page.locator('#view-jobs').innerText()).includes(sent.body.label));
  await selectDomain(page, DOMAIN); await open(); await step('setup');
  assert.equal(await page.locator('[data-fleet-installation]').count(), 0, 'previous discoveries cleared');
  // Late HTTP results must be aborted and fenced on Domain switch and logout.
  for (const action of ['domain', 'logout']) {
    await configure(); await page.locator('#jobs-input').fill(INPUT); state.hold = '/estimate';
    await page.locator('#jobs-estimate').click();
    await fixture.waitFor(() => fixture.state.exchanges.some(e => e.path === '/v1/jobs/estimate' && e.held && !e.finished && !e.aborted));
    const pending = fixture.state.exchanges.filter(e => e.path === '/v1/jobs/estimate' && e.held).at(-1);
    const cancelCount = state.cancellations.length;
    if (action === 'domain') await selectDomain(page, OTHER);
    else { await page.locator('#logout').click(); await page.waitForFunction(() => document.querySelector('#session-status')?.textContent === 'Signed out'); }
    await fixture.waitFor(() => pending.aborted); fixture.release();
    assert.equal(state.cancellations.length, cancelCount, 'closing never cancels a server job');
    if (action === 'logout') await login('jobs-b@example.test', OTHER);
    await open(); await step('setup');
    assert.equal(await page.locator('[data-fleet-installation]').count(), 0, 'previous discoveries cleared');
    const visible = await page.locator('#view-jobs').innerText();
    for (const old of [DOMAIN, INPUT, COST, CONFIG.installationId, ambiguous.body.label]) assert.ok(!visible.includes(old), 'no previous session/Domain results');
    assert.equal(await page.locator('[data-job-output]').count(), 0); assert.equal(await page.locator('#jobs-confirm').count(), 0);
    await selectDomain(page, DOMAIN);
  }
  await noRetry();
  assert.deepEqual(await page.evaluate(() => ({ local: Object.keys(localStorage), session: Object.keys(sessionStorage) })), { local: ['core-explorer.client-id'], session: [] });
  for (const secret of ['synthetic-password', 'synthetic-user', 'synthetic-refresh', 'synthetic-backend-secret']) assert.ok(!logs.join('\n').includes(secret));
  assert.deepEqual(state.violations, []); assert.deepEqual(forbidden, []); assert.deepEqual(errors, []);
  assert.ok(state.grants.every(grant => grant.scopes.includes('domain-data:rw')));
  assert.ok(!fixture.state.requests.some(request => /\/tasks|relay-bookings|p2p\//.test(request)), 'jobs do not start peers or workers');
  console.log('PASS real-WASM jobs browser against synthetic HTTP provider; no live worker execution claimed.');
} finally {
  try { await browser?.close(); } finally {
    try { if (vite.exitCode === null && vite.signalCode === null) { const exited = once(vite, 'exit'); vite.kill('SIGTERM'); await exited; } }
    finally { await fixture.close(); }
  }
}
