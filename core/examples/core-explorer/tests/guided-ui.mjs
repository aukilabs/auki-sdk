import assert from 'node:assert/strict';

export async function assertChapter(page, selected) {
  for (const view of ['overview', 'data', 'networking']) {
    const button = page.locator(`[data-view="${view}"]`);
    assert.equal(await page.locator(`#view-${view}`).isVisible(), view === selected, `${view} visibility`);
    const current = await button.getAttribute('aria-current');
    assert.equal(current !== null && current !== 'false', view === selected, `${view} accessible selection`);
  }
}

export async function chapter(page, view, keyboard = false) {
  const button = page.locator(`[data-view="${view}"]`);
  if (keyboard) {
    // Reach the button through normal sequential keyboard navigation.
    for (let i = 0; i < 100 && !await button.evaluate(el => el === document.activeElement); i++) await page.keyboard.press('Tab');
    assert.ok(await button.evaluate(el => el === document.activeElement), `${view} reachable by Tab`);
    await page.keyboard.press('Enter');
  } else await button.click();
  await assertChapter(page, view);
}

export async function openDetails(page, selector) {
  const details = page.locator(selector);
  if (!await details.evaluate(el => el.open)) await details.locator(':scope > summary').click();
  assert.ok(await details.evaluate(el => el.open));
}

// Reveal technical values using exactly the same navigation/disclosures as a reader.
export async function reveal(page, selector) {
  const target = page.locator(selector);
  const view = await target.evaluate(el => el.closest('[id^="view-"]')?.id.slice(5));
  if (view) await chapter(page, view);
  const step = await target.evaluate(el => el.closest('[data-step]')?.getAttribute('data-step'));
  if (step === 'discover' && await page.locator('[data-step="discover"] .step-body').isHidden()
      && await page.locator('#net-reselect').isVisible()) await page.locator('#net-reselect').click();
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
  for (const step of ['connect', 'discover', 'diagnostic']) {
    assert.equal(await page.locator(`[data-step="${step}"] .step-body`).isVisible(), step === selected, `${step} body visibility`);
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
  return message.type() === 'error' && !/^Failed to load resource: (?:the server responded with a status of (?:401|403)\b|net::ERR_(?:ABORTED|CONNECTION_CLOSED|CONNECTION_RESET|CONNECTION_REFUSED)\b)/.test(message.text());
}
