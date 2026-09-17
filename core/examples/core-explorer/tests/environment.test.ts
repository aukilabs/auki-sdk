import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import ts from 'typescript';
import { environmentLabel } from '../src/presentation.ts';
import { endpoint, ReadLane } from '../src/safety.ts';
import { ScreenHistory } from '../src/screens.ts';

const html = readFileSync(new URL('../index.html', import.meta.url), 'utf8');
const defaults = ['https://api.dev.aukiverse.com/', 'https://dds.dev.aukiverse.com/', 'https://dms.dev.aukiverse.com/v1/'];
const ids = ['api', 'dds', 'dms'];

test('editable HTML defaults are the exact approved Dev bases', () => {
  for (const [index, id] of ids.entries()) {
    const input = html.match(new RegExp(`<input id="${id}"[^>]+>`))![0];
    assert.ok(input.includes(`value="${defaults[index]}"`));
    assert.doesNotMatch(input, /disabled|readonly/);
  }
  assert.match(html, /id="environment"[^>]*>Dev · sign in to contact these services</);
});

test('environment labels distinguish exact Dev, loopback and custom or mixed bases', () => {
  assert.equal(environmentLabel(defaults), 'Dev');
  assert.equal(environmentLabel(defaults.map(endpoint)), 'Dev');
  assert.equal(environmentLabel(['http://localhost:1/', 'http://127.0.0.1:2/', 'http://[::1]:3/v1/']), 'Local fixtures / synthetic test data');
  for (const urls of [
    [defaults[0], defaults[1], 'https://dms.dev.aukiverse.com/'],
    ['http://127.0.0.1:18114', defaults[1], defaults[2]],
    [defaults[1], defaults[0], defaults[2]],
    defaults.map(url => url.replace('.dev.', '.custom.')),
    [defaults[0] + 'other/', defaults[1], defaults[2]],
    ['https://api.dev.aukiverse.com.example.test/', defaults[1], defaults[2]],
  ]) assert.equal(environmentLabel(urls), 'Custom / mixed environment');
  for (const invalid of ['http://api.dev.aukiverse.com/', 'https://user:secret@example.test/', defaults[0] + '?x=1', 'invalid']) {
    assert.throws(() => environmentLabel([invalid, defaults[1], defaults[2]]));
  }
});

test('actual main startup and settings handlers do not sign in or read services', async () => {
  const nodes = new Map<string, any>();
  const node = (id: string) => {
    if (!nodes.has(id)) nodes.set(id, { value: ids.includes(id) ? defaults[ids.indexOf(id)] : '', textContent: '', disabled: false });
    return nodes.get(id);
  };
  let logins = 0, reads = 0;
  const source = readFileSync(new URL('../src/main.ts', import.meta.url), 'utf8');
  const tree = ts.createSourceFile('main.ts', source, ts.ScriptTarget.Latest, true);
  const code = ts.transpileModule(tree.statements.filter(n => !ts.isImportDeclaration(n)).map(n => n.getText(tree)).join('\n'), { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText;
  runInNewContext(code, {
    document: { getElementById: node, addEventListener() {} }, window: { addEventListener() {} },
    ScreenHistory, ReadLane, endpoint, environmentLabel, setTimeout, clearTimeout,
    Connection: class { beforeClose = async () => {}; async accept(pending: Promise<unknown>) { await pending; return false; } },
    networkingUI: () => ({}), uploadUI: () => ({}), jobsUI: () => ({}),
    login: async () => { logins++; },
    fetch: () => { reads++; throw new Error('Unexpected service IO'); },
  });
  assert.equal(node('environment').textContent, 'Dev · sign in to contact these services');
  for (const id of ids) { node(id).value = 'http://127.0.0.1:18114'; node(id).oninput(); }
  assert.match(node('environment').textContent, /^Local fixtures \/ synthetic test data/);
  node('api').value = defaults[0]; node('api').oninput();
  assert.match(node('environment').textContent, /^Custom \/ mixed environment/);
  node('api').value = 'invalid'; node('api').oninput();
  assert.match(node('environment').textContent, /^Invalid endpoint/);
  assert.equal(logins, 0); assert.equal(reads, 0);
  node('api').value = 'http://127.0.0.1:18114';
  await node('login').onsubmit({ preventDefault() {} });
  assert.equal(logins, 1, 'only explicit form submission invokes login');
  assert.equal(reads, 0);
});

test('each repository browser harness overrides all service URLs before sign-in', () => {
  for (const file of ['browser.mjs', 'upload-browser.mjs', 'jobs-browser.mjs', 'fleet-browser.mjs', 'network-browser.mjs']) {
    const source = readFileSync(new URL(file, import.meta.url), 'utf8');
    const beforeSignIn = source.slice(0, source.indexOf("locator('#signin').click()"));
    assert.match(beforeSignIn, /for\s*\(const id of \['api',\s*'dds'(?:,\s*'dms')?\]\)[^\n]*\.fill\(fixture.base\)/, file);
    assert.ok(/\['api',\s*'dds',\s*'dms'\]/.test(beforeSignIn) || /locator\('#dms'\)\.fill\(fixture.base \+ '\/v1\/'\)/.test(beforeSignIn), `${file}: explicit DMS fixture`);
    assert.match(beforeSignIn, /route\.abort\(\)/, `${file}: nonlocal requests blocked`);
  }
});
