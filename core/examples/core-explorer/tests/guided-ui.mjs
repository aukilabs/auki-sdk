import assert from 'node:assert/strict';

export async function assertChapter(page, selected) {
  for (const view of ['overview', 'data', 'portals', 'poses', 'networking', 'jobs', 'fleet']) {
    assert.equal(await page.locator(`#view-${view}`).isVisible(), view === selected, `${view} visibility`);
  }
  const group = ['portals', 'poses'].includes(selected) ? 'overview' : selected;
  for (const view of ['data', 'jobs', 'fleet', 'overview', 'networking']) {
    const current = await page.locator(`[data-view="${view}"]`).getAttribute('aria-current');
    assert.equal(current === 'page', view === group, `${view} accessible selection`);
  }
}

export async function chapter(page, view, keyboard = false) {
  if (['portals', 'poses'].includes(view)) await chapter(page, 'overview', keyboard);
  const button = page.locator(['portals', 'poses'].includes(view) ? `[data-go="${view}"]` : `[data-view="${view}"]`);
  if (keyboard) {
    for (let i = 0; i < 100 && !await button.evaluate(el => el === document.activeElement); i++) await page.keyboard.press('Tab');
    assert.ok(await button.evaluate(el => el === document.activeElement), `${view} reachable by Tab`);
    await page.keyboard.press('Enter');
  } else await button.click();
  await assertChapter(page, view);
}

export async function back(page) {
  const button = page.locator('[data-back]:visible').first();
  await button.click();
}

export async function go(page, view) {
  if (await page.locator(`#view-${view}`).isVisible()) return;
  if (['portals', 'poses'].includes(view)) return chapter(page, view);
  const primary = page.locator(`[data-view="${view}"]:visible`);
  if (await primary.count()) return chapter(page, view);
  if (view === 'record') { await chapter(page, 'data'); await page.locator('#record-jump').click(); return; }
  if (view === 'upload') { await chapter(page, 'data'); await page.locator('#open-upload').click(); return; }
  let action = page.locator(`[data-go="${view}"]:visible`).first();
  if (!await action.count() && await page.locator('[data-back]:visible').count()) {
    await back(page);
    action = page.locator(`[data-go="${view}"]:visible`).first();
  }
  if (!await action.count() && await page.locator('[data-view="data"]:visible').count()) {
    await chapter(page, ['filters', 'record', 'preview', 'upload'].includes(view) ? 'data' : 'overview');
    action = page.locator(`[data-go="${view}"]:visible`).first();
  }
  await action.click();
  assert.ok(await page.locator(`#view-${view}`).isVisible(), `${view} focused screen`);
}

export async function openDetails(page, selector) {
  const routes = { '#connection-settings': 'settings', '#domain-controls': 'domains', '#advanced-filters': 'filters' };
  if (routes[selector]) {
    await go(page, routes[selector]);
    if (selector !== '#advanced-filters') return;
  }
  const details = page.locator(selector);
  if (!await details.evaluate(el => el.open)) await details.locator(':scope > summary').click();
  assert.ok(await details.evaluate(el => el.open));
}

// Navigate through declared user-facing hooks; never unhide DOM or bypass handlers.
export async function reveal(page, selector) {
  const target = page.locator(selector);
  if (selector === '#record' || selector === '#metadata') {
    await go(page, selector === '#record' ? 'record' : 'overview');
    await page.locator(selector === '#record' ? '#record-technical' : '#domain-technical').click();
    return;
  }
  const view = await target.evaluate(el => el.closest('[id^="view-"]')?.id.slice(5));
  if (view && !await target.isVisible()) await go(page, view);
  const networkScreen = await target.evaluate(el => el.closest('[data-network-screen]')?.getAttribute('data-network-screen'));
  if (networkScreen && !await target.isVisible()) {
    if (networkScreen === 'manual') {
      if (!await page.locator('#net-show-manual').isVisible()) {
        if (await page.locator('#net-result-back').isVisible()) await page.locator('#net-result-back').click();
        if (await page.locator('#net-reselect').isVisible()) await page.locator('#net-reselect').click();
        else if (await page.locator('#net-back').isVisible()) await page.locator('#net-back').click();
      }
      await page.locator('#net-show-manual').click();
    }
    else if (networkScreen === 'technical') await page.locator('#net-show-technical').click();
    else if (networkScreen === 'diagnostic' && await page.locator('#net-result-back').isVisible()) await page.locator('#net-result-back').click();
    else if (await page.locator('#net-back').isVisible()) await page.locator('#net-back').click();
    if (networkScreen === 'discover' && !await target.isVisible()) {
      if (await page.locator('#net-result-back').isVisible()) await page.locator('#net-result-back').click();
      await page.locator('#net-reselect').click();
    }
  }
  const ancestors = target.locator('xpath=ancestor::details');
  for (let i = 0; i < await ancestors.count(); i++) {
    const details = ancestors.nth(i);
    if (!await details.evaluate(el => el.open)) await details.locator(':scope > summary').click();
  }
  const nested = target.locator('details');
  for (let i = 0; i < await nested.count(); i++) {
    const details = nested.nth(i);
    if (!await details.evaluate(el => el.open)) await details.locator(':scope > summary').click();
  }
}

export async function assertNetworkStep(page, selected) {
  for (const step of ['connect', 'discover', 'diagnostic', 'manual', 'technical', 'result']) {
    assert.equal(await page.locator(`[data-network-screen="${step}"]`).isVisible(), step === selected, `${step} screen visibility`);
  }
  assert.ok(await page.locator('#net-stop').isVisible(), 'stop remains accessible');
}

export async function selectDomain(page, id) {
  await reveal(page, '#domain-id');
  await page.locator('#domain-id').fill(id);
  await page.locator('#manual button').click();
}

export async function assertLocalFonts(page) {
  await page.evaluate(() => document.fonts.ready);
  const fonts = await page.evaluate(() => [...document.fonts].filter(font => font.status === 'loaded').map(font => font.family.replaceAll('"', '').replaceAll("'", '')));
  for (const family of ['Space Grotesk', 'DM Sans']) assert.ok(fonts.includes(family), `${family} actually loaded`);
  const resources = await page.evaluate(() => performance.getEntriesByType('resource').filter(entry => /\.(?:ttf|woff2?)(?:\?|$)/i.test(entry.name)).map(entry => entry.name));
  assert.ok(resources.length >= 2, 'local font files requested');
  for (const resource of resources) {
    const url = new URL(resource);
    assert.equal(url.origin, new URL(page.url()).origin);
    assert.ok(url.pathname.startsWith('/brand/fonts/'));
  }
  assert.match(await page.locator('body').evaluate(el => getComputedStyle(el).fontFamily), /DM Sans/);
  assert.match(await page.locator('h1').first().evaluate(el => getComputedStyle(el).fontFamily), /Space Grotesk/);
}

export async function assertUsable(page, selectors) {
  assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'page fits viewport');
  for (const selector of selectors) {
    const control = page.locator(selector);
    await control.scrollIntoViewIfNeeded();
    const box = await control.boundingBox();
    const width = page.viewportSize().width;
    assert.ok(box && box.width >= 24 && box.height >= 24 && box.x >= 0 && box.x + box.width <= width + 1, `${selector} usable bounds`);
    await control.click({ trial: true });
  }
}

export function unexpectedConsoleError(message) {
  return message.type() === 'error' && !/^Failed to load resource: (?:the server responded with a status of (?:401|403|409)\b|net::ERR_(?:ABORTED|EMPTY_RESPONSE|CONNECTION_CLOSED|CONNECTION_RESET|CONNECTION_REFUSED)\b)/.test(message.text());
}
