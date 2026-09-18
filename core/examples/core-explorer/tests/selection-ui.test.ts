import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import ts from 'typescript';

// Execute the actual UI functions/event registrations with a minimal DOM. This
// offline unit harness does not alter production code or the real-browser SDK.
const source = readFileSync(new URL('../src/main.ts', import.meta.url), 'utf8');
const tree = ts.createSourceFile('main.ts', source, ts.ScriptTarget.Latest, true);
const selected = tree.statements.filter(node =>
  ts.isFunctionDeclaration(node) && ['selectDomain', 'loadDomains'].includes(node.name?.text ?? '') ||
  ts.isExpressionStatement(node) && node.getText(tree).startsWith("$('manual').onsubmit"));
const presentation = ts.createSourceFile('presentation.ts', readFileSync(new URL('../src/presentation.ts', import.meta.url), 'utf8'), ts.ScriptTarget.Latest, true);
const row = presentation.statements.find(node => ts.isFunctionDeclaration(node) && node.name?.text === 'recordRow')!;
const code = ts.transpileModule(row.getText(presentation).replace('export ', '') + '\n' + selected.map(node => node.getText(tree)).join('\n'), { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText;
const checkpoint = () => new Promise<void>(resolve => setImmediate(resolve));
function deferred() {
  let resolve!: (value?: unknown) => void, reject!: (error: unknown) => void;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
function harness() {
  const nodes = new Map<string, any>();
  const $ = (id: string) => {
    if (!nodes.has(id)) nodes.set(id, { value: '', textContent: '', replaceChildren() {}, append(child: any) { this.child = child; } });
    return nodes.get(id);
  };
  const calls: ReturnType<typeof deferred>[] = [];
  const session = { domains: () => ({ list: async () => ({ domains: [{ id: 'A', name: 'Alpha' }], offset: 0, total: 1, limit: 10 }) }) };
  const context: any = {
    $, text: (id: string, value: string) => { $(id).textContent = value; }, input: () => 'A', uuid: (value: string) => value,
    disabled: () => {}, selection: 0, domainId: '', domainName: '', summaries: [], offset: 0,
    connection: { session, select: () => { const pending = deferred(); calls.push(pending); return pending.promise; } },
    clearSelection: () => { context.selection++; context.domainId = ''; },
    redact: (value: unknown) => value, inspect: JSON.stringify, facts: () => [], safeError: () => 'Safe read error.',
    navigation: { reset: () => {} }, renderView: () => {}, showView: () => {},
    jobs: { refreshContext: () => {} }, networking: { refresh: () => {} }, refreshSelected: () => {},
    document: { createElement: () => ({ append() {} }) },
    lanes: { domains: { run: async (read: any, success: any) => success(await read()) } },
  };
  runInNewContext(code, context);
  return { context, $, calls };
}

for (const entry of ['listed', 'manual']) test(`${entry} Domain event catches asynchronous failure and displays only a safe message`, async () => {
  const { context, $, calls } = harness();
  if (entry === 'listed') { context.loadDomains(); await checkpoint(); $('domains').child.onclick(); }
  else $('manual').onsubmit({ preventDefault() {} });
  calls[0].reject(new Error('PRIVATE_BACKEND_BODY'));
  await checkpoint();
  const displayed = `${$('domain-status').textContent} ${$('notice').textContent}`;
  assert.match(displayed, /Domain.*failed|Unable to select|Could not select/);
  assert.ok(!displayed.includes('PRIVATE_BACKEND_BODY'));
});

for (const next of ['selection', 'logout']) test(`stale Domain failure cannot overwrite newer ${next}`, async () => {
  const { context, $, calls } = harness();
  const old = context.selectDomain('A');
  // Observe the promise even on the unfixed implementation.
  const observed = Promise.allSettled([old]);
  if (next === 'selection') { const newer = context.selectDomain('B'); calls[1].resolve(); await newer; }
  else { context.clearSelection(); context.connection.session = undefined; }
  $('domain-status').textContent = 'new state'; $('notice').textContent = 'new notice';
  calls[0].reject(new Error('PRIVATE_BACKEND_BODY'));
  const outcomes = await observed; await checkpoint();
  assert.equal(outcomes[0].status, 'fulfilled', 'stale failures must also be consumed');
  assert.equal($('domain-status').textContent, 'new state');
  assert.equal($('notice').textContent, 'new notice');
});
