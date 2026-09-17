import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, existsSync } from 'node:fs';
const html = readFileSync(new URL('../index.html', import.meta.url), 'utf8');
const css = readFileSync(new URL('../src/style.css', import.meta.url), 'utf8');

test('all workspaces keep unique DOM hooks and the four accessible navigation targets', () => {
  const ids = [...html.matchAll(/\bid="([^"]+)"/g)].map(match => match[1]);
  assert.equal(new Set(ids).size, ids.length);
  assert.deepEqual([...html.matchAll(/data-view="([^"]+)"/g)].map(match => match[1]), ['data', 'jobs', 'overview', 'networking']);
  for (const route of ['domains', 'settings', 'filters', 'record', 'preview', 'technical', 'upload', 'portals', 'poses']) assert.ok(ids.includes(`view-${route}`));
  assert.match(html, /Exact name<input id="name"/);
  assert.doesNotMatch(html, /chapter-num|class="paper"|Understand the space|Look inside/);
});
test('local font references resolve and responsive scroll escape paths remain present', () => {
  for (const [, asset] of css.matchAll(/url\('([^']+)'\)/g)) assert.ok(existsSync(new URL(`../public${asset}`, import.meta.url)), asset);
  assert.match(css, /max-width:760px/);
  assert.match(css, /max-height:600px/);
  assert.match(css, /\.view\{[^}]*overflow:auto/);
  assert.match(css, /\[hidden\]\{display:none!important/);
});
