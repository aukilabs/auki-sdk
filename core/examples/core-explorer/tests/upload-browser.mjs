// Run after the parent builds the real SDK/app: node tests/upload-browser.mjs.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { readFile, mkdir } from 'node:fs/promises';
import { chromium } from 'playwright';
import { startFixture, DOMAIN, OTHER, MAX_UPLOAD } from './fixture.mjs';
import { go, back, chapter, reveal, selectDomain, assertUsable, unexpectedConsoleError } from './guided-ui.mjs';
const fixture = await startFixture();
const vite = spawn(process.execPath, ['node_modules/vite/bin/vite.js', 'preview', '--host', '127.0.0.1', '--port', '18116', '--strictPort'], { stdio: 'pipe' });
let browser;
const artifacts = new URL('../test-artifacts/', import.meta.url);
try {
  let ready = false;
  for (let i = 0; i < 100; i++) {
    try { ready = (await fetch('http://127.0.0.1:18116')).ok; } catch {}
    if (ready) break;
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  assert.ok(ready, 'prebuilt preview is available');
  browser = await chromium.launch({ headless: true, args: ['--no-sandbox', '--disable-dev-shm-usage'] });
  const context = await browser.newContext({ acceptDownloads: true, serviceWorkers: 'block', viewport: { width: 1440, height: 900 } });
  const forbidden = [], errors = [], logs = [];
  const allowed = new Set(['http://127.0.0.1:18116', fixture.base, fixture.dataBase]);
  await context.route('**/*', route => {
    if (!allowed.has(new URL(route.request().url()).origin)) { forbidden.push(route.request().url()); return route.abort(); }
    return route.continue();
  });
  const page = await context.newPage(); page.setDefaultTimeout(10000);
  page.on('pageerror', error => errors.push(error.message));
  page.on('console', message => { logs.push(message.text()); if (unexpectedConsoleError(message)) errors.push(message.text()); });
  // Observe JS submissions before WASM/app initialization without altering requests.
  await page.addInitScript(() => {
    const original = window.fetch;
    window.__uploadSubmissions = [];
    window.fetch = function(input, init) {
      const url = input instanceof Request ? input.url : String(input);
      const method = init?.method ?? (input instanceof Request ? input.method : 'GET');
      if (method.toUpperCase() === 'POST' && /\/api\/v1\/domains\/[^/]+\/data$/.test(new URL(url, location.href).pathname)) {
        window.__uploadSubmissions.push(url);
      }
      return original.apply(this, arguments);
    };
  });
  await page.goto('http://127.0.0.1:18116');
  assert.equal(fixture.state.requests.length, 0);
  await go(page, 'settings');
  for (const id of ['api', 'dds', 'dms']) await page.locator(`#${id}`).fill(fixture.base);
  await back(page);
  const login = async (email = 'account-a@example.test', domain = DOMAIN) => {
    await page.locator('#email').fill(email);
    await page.locator('#password').fill('synthetic-password'); await page.locator('#signin').click();
    await page.waitForFunction(() => document.querySelector('#domains')?.textContent.includes('Synthetic lab'));
    await selectDomain(page, domain);
    if (domain === DOMAIN) await page.waitForFunction(() => document.querySelector('#records')?.textContent.includes('Lab sample'));
  };
  await login();
  const step = async name => {
    await page.locator(`[data-upload-step="${name}"]:visible`).waitFor();
    assert.equal(await page.locator('[data-upload-step]:visible').count(), 1, 'one upload step visible');
  };
  const bytes = Buffer.from([0, 255, 128, 13, 10, 34, 92, 60, 62, 0, 42]);
  const choose = async (buffer = bytes, type = null) => {
    await chapter(page, 'data'); await go(page, 'upload');
    if (await page.locator('#upload-new').isVisible()) await page.locator('#upload-new').click();
    await step('choose');
    await page.locator('#upload-file').setInputFiles({ name: 'fixture.bin', mimeType: 'application/octet-stream', buffer });
    if (type !== null) await page.locator('#upload-type').fill(type);
  };
  const review = async () => {
    await choose(); await page.locator('#upload-review').click(); await step('review');
    const text = await page.locator('#view-upload').innerText();
    assert.ok(text.includes(DOMAIN), 'review names destination Domain UUID');
    assert.match(text, /Synthetic lab|Known Domain/);
    assert.match(text, /Local fixtures|synthetic test data/i);
    assert.match(text, /11\s*(?:bytes|B)\b/i);
    const target = (await page.locator('#upload-target').inputValue()).trim(); assert.ok(target);
    assert.equal(await page.locator('#upload-confirm').isEnabled(), true);
    return target;
  };
  const finish = async () => { await step('result'); return page.locator('#upload-status').innerText(); };
  let confirmations = 0;
  const confirm = async () => { confirmations++; await page.locator('#upload-confirm').click(); };
  const assertNoRetry = async () => {
    // A quiet window catches delayed automatic retries, not just the first response.
    await page.waitForTimeout(700);
    assert.equal(await page.evaluate(() => window.__uploadSubmissions.length), confirmations, 'exactly one JS write submission per explicit confirmation; no UI retry');
  };
  // Default type works without any input edit, and review starts at the destination.
  await choose(bytes, null);
  assert.equal(await page.locator('#upload-type').inputValue(), 'core-explorer.file.v1');
  await page.locator('#upload-review').click(); await step('review');
  assert.equal(await page.locator('#view-upload').evaluate(el => el.scrollTop), 0);
  assert.equal(await page.locator('#upload-heading').evaluate(el => el === document.activeElement), true);
  await page.locator('#upload-cancel').click();
  // Back must not remove native controls from the Tab sequence.
  await chapter(page, 'data'); await go(page, 'filters'); await back(page);
  assert.equal(await page.locator('[data-go="filters"]').evaluate(el => el.tabIndex), 0);
  // Invalid local inputs must not even obtain upload server info.
  for (const [buffer, type] of [[Buffer.alloc(MAX_UPLOAD + 1), 'fixture.binary.v1'], [bytes, 'bad/type']]) {
    await choose(buffer, type); const before = fixture.state.requests.length;
    if (await page.locator('#upload-review').isEnabled()) await page.locator('#upload-review').click();
    assert.equal(await page.locator('[data-upload-step="review"]:visible').count(), 0);
    assert.equal(fixture.state.requests.length, before, 'invalid upload rejected before SDK request');
    await page.locator('#upload-back').click();
  }
  // A fresh session resets invalid edits; successful uploads use the untouched default.
  await page.locator('#logout').click();
  await page.waitForFunction(() => document.querySelector('#session-status')?.textContent === 'Signed out');
  await login();
  await review();
  const beforeReviewCancel = fixture.state.uploads.length;
  await page.locator('#upload-cancel').click(); await step('choose');
  assert.equal(fixture.state.uploads.length, beforeReviewCancel, 'cancelling review sends nothing');
  // A successful write response is insufficient until a separate SDK metadata GET finishes.
  fixture.state.holdVerification = true;
  const target = await review(); const start = fixture.state.exchanges.length;
  await confirm();
  await fixture.waitFor(() => fixture.state.exchanges.slice(start).some(e => e.method === 'GET' && e.held));
  const saved = fixture.state.uploads.at(-1);
  assert.ok(saved.stored); assert.equal(saved.domain_id, DOMAIN);
  assert.ok(target.includes(saved.name)); assert.equal(saved.data_type, 'core-explorer.file.v1');
  assert.deepEqual(saved.bytes, bytes);
  assert.equal(await page.locator('[data-upload-step="result"]:visible').count(), 0, 'write response alone is not verified');
  fixture.releaseVerification(); assert.match(await finish(), /^Upload verified/i);
  assert.ok(fixture.state.requests.includes(`GET /api/v1/domains/${DOMAIN}/data/${saved.record.id}`));
  await assertNoRetry();
  await page.locator('#upload-metadata').click();
  assert.equal(await page.locator('#view-upload').isVisible(), false);
  const uploadedMetadata = JSON.parse(await page.locator('#technical-content').textContent());
  assert.equal(uploadedMetadata.id, saved.record.id); assert.equal(uploadedMetadata.name, saved.name);
  await back(page); await step('result');
  const assertAccountReset = async oldTarget => {
    await page.locator('#logout').click();
    await page.waitForFunction(() => document.querySelector('#session-status')?.textContent === 'Signed out');
    await login('account-b@example.test', OTHER);
    await go(page, 'upload'); await step('choose');
    assert.equal(await page.locator('#upload-target').inputValue(), '');
    assert.equal(await page.locator('#upload-file').inputValue(), '');
    assert.equal(await page.locator('#upload-type').inputValue(), 'core-explorer.file.v1');
    const retained = await page.locator('#view-upload').textContent();
    for (const old of [oldTarget, DOMAIN, 'fixture.bin', saved.record.id]) assert.ok(!retained.includes(old), 'no account A details in account B upload');
    assert.equal(await page.locator('#technical-content').textContent(), '');
    await page.locator('#logout').click();
    await page.waitForFunction(() => document.querySelector('#session-status')?.textContent === 'Signed out');
    await login();
  };
  await mkdir(artifacts, { recursive: true });
  for (const [width, height] of [[1440, 900], [390, 844], [320, 568], [900, 500]]) {
    await page.setViewportSize({ width, height });
    await assertUsable(page, ['#upload-back', '#upload-new']);
    await page.screenshot({ path: new URL(`upload-${width}x${height}.png`, artifacts).pathname, fullPage: true });
  }
  await page.setViewportSize({ width: 1440, height: 900 });
  // Download through the app/SDK, not a synthetic browser fetch.
  await chapter(page, 'data');
  await page.locator('#records button').filter({ has: page.getByText(saved.name, { exact: true }) }).click();
  await reveal(page, '#download'); const downloading = page.waitForEvent('download'); await page.locator('#download').click();
  const download = await downloading;
  assert.equal(download.suggestedFilename(), `${saved.record.id}.bin`);
  assert.deepEqual(await readFile(await download.path()), bytes);
  await assertAccountReset(target);
  for (const mode of ['denyWrite', 'collideWrite', 'failWrite', 'dropWriteResponse', 'denyVerification']) {
    fixture.state[mode] = true;
    const target = await review(); const before = fixture.state.uploads.length;
    await confirm(); const status = await finish();
    assert.doesNotMatch(status, /^Upload verified/i);
    assert.match(status, /denied|collision|exist|fail|error|uncertain|verify|verification|check|permission/i);
    assert.ok((await page.locator('#upload-target').inputValue()).includes(target), 'failed/ambiguous target remains available');
    await assertNoRetry();
    const attempts = fixture.state.uploads.slice(before);
    assert.ok(attempts.length >= 1, 'server observed the explicit submission');
    for (const attempt of attempts) {
      assert.equal(attempt.name, target); assert.equal(attempt.domain_id, DOMAIN);
      assert.equal(attempt.data_type, 'core-explorer.file.v1'); assert.deepEqual(attempt.bytes, bytes);
    }
    assert.ok(attempts.filter(attempt => attempt.stored).length <= 1, 'retransmissions never replace the fixture record');
    const stored = fixture.records.filter(record => record.name === target);
    assert.ok(stored.length <= 1, 'at most one fixture record for the unique target');
    if (mode === 'dropWriteResponse' || mode === 'denyVerification') {
      assert.match(status, /uncertain/i);
      assert.equal(stored.length, 1, 'uncertain outcome retains server record');
      assert.deepEqual(fixture.contents.get(stored[0].id), bytes);
    }
    if (mode === 'denyWrite') assert.ok(fixture.records.some(record => record.name === 'Lab sample'), 'read success does not confer write permission');
    fixture.state[mode] = false;
    if (mode === 'dropWriteResponse') await assertAccountReset(target);
  }
  // Cancel while the server retains a completed write but has not responded.
  for (const action of ['cancel', 'domain', 'logout']) {
    fixture.state.holdWrites = true; const target = await review(); const before = fixture.state.uploads.length;
    await confirm();
    await fixture.waitFor(() => fixture.state.uploads.length === before + 1 && fixture.state.uploads.at(-1).exchange.held);
    const pending = fixture.state.uploads.at(-1); assert.ok(pending.stored);
    if (action === 'cancel') await page.locator('#upload-cancel').click();
    else if (action === 'domain') await selectDomain(page, OTHER);
    else await page.locator('#logout').click();
    await fixture.waitFor(() => pending.exchange.aborted);
    fixture.releaseWrites(); await assertNoRetry();
    assert.ok(fixture.contents.has(pending.record.id), 'cancellation never promises server rollback');
    if (action === 'cancel') {
      assert.ok((await page.locator('#upload-target').inputValue()).includes(target), 'cancelled target retained');
      assert.match(await page.locator('#upload-status').textContent(), /cancel|uncertain|check/i);
    } else if (action === 'domain') {
      assert.equal(await page.locator('#view-upload').isVisible(), false);
      await selectDomain(page, DOMAIN);
    } else {
      await page.waitForFunction(() => document.querySelector('#session-status')?.textContent === 'Signed out');
      assert.equal(await page.locator('#workspace').isVisible(), false);
      await login('account-b@example.test', OTHER); await go(page, 'upload'); await step('choose');
      assert.equal(await page.locator('#upload-target').inputValue(), '');
      assert.equal(await page.locator('#upload-file').inputValue(), '');
      const retained = await page.locator('#view-upload').textContent();
      for (const old of [target, DOMAIN, 'fixture.bin', pending.record.id]) assert.ok(!retained.includes(old));
      assert.equal(await page.locator('#technical-content').textContent(), '');
    }
  }
  const storage = await page.evaluate(() => ({ local: Object.keys(localStorage), session: Object.keys(sessionStorage) }));
  assert.deepEqual(storage, { local: ['core-explorer.client-id'], session: [] });
  for (const secret of ['synthetic-password', 'synthetic-user', 'synthetic-refresh', 'synthetic-backend-secret']) assert.ok(!logs.join('\n').includes(secret));
  assert.deepEqual(forbidden, []); assert.deepEqual(errors, []);
  assert.ok(!fixture.state.requests.some(request => /^(?:PUT|DELETE) /.test(request)), 'no overwrite/delete');
  console.log('PASS real-SDK upload: binary roundtrip, independent metadata verification, limits, denial/collision/network ambiguity, no retry, cancellation/Domain/logout fencing.');
} finally {
  try { await browser?.close(); } finally {
    try {
      if (vite.exitCode === null && vite.signalCode === null) { const exited = once(vite, 'exit'); vite.kill('SIGTERM'); await exited; }
    } finally { await fixture.close(); }
  }
}
