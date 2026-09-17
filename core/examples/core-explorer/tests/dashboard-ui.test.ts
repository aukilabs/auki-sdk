import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import ts from 'typescript';
const source = readFileSync(new URL('../src/jobs-ui.ts', import.meta.url), 'utf8');
const tree = ts.createSourceFile('jobs-ui.ts', source, ts.ScriptTarget.Latest, true);
const selected: string[] = [];
function visit(node: ts.Node) {
  if (ts.isFunctionDeclaration(node) && ['activateReads', 'previewRecord', 'loadPicker', 'invalidateReads', 'evidence', 'enterVisible', 'close'].includes(node.name?.text ?? '')) selected.push(node.getText(tree));
  if (ts.isExpressionStatement(node) && node.getText(tree).startsWith('new MutationObserver')) selected.push(node.getText(tree));
  if (ts.isMethodDeclaration(node) && node.name.getText(tree) === 'refreshContext') selected.push(`globalThis.refresh = ({${node.getText(tree)}}).refreshContext;`);
  ts.forEachChild(node, visit);
}
visit(tree);
const code = ts.transpileModule(selected.join('\n'), { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText;
const tick = () => new Promise(resolve => setImmediate(resolve));
function harness() {
  const nodes = new Map<string, any>();
  let observe!: () => void, reads = 0, discoveries = 0, lists = 0;
  const context = { domainId: 'a', session: {}, data: {}, environment: 'fixture' };
  const ui: any = { generation: 0, pickerVersion: 0, previewVersion: 0, pickerStarted: false, entered: false, ready: true, discoveryReady: false, busy: false, polls: 0,
    screen: 'detail', override: undefined, draftInput: '', draftRole: 'compute', closing: Promise.resolve(), context, connection: { session: context.session },
    names: new Map(), previews: new Map(), previewStates: new Map(), summaries: new Map(), section: { hidden: false },
    getContext: () => ui.context, selectedInput() {}, stopPolling() {}, schedule() {}, render() {},
    text(id: string, text: string) { ui.get(id).textContent = text; }, get(id: string) { if (!nodes.has(id)) nodes.set(id, { textContent: '', replaceChildren() {}, append() {} }); return nodes.get(id); },
    button: () => ({}), field: () => ({}), redact: (v: unknown) => v,
    records: { preview: async () => { reads++; return { record: { name: 'Named result' }, text: 'result' }; }, list: async () => { lists++; return []; }, close: async () => {} },
    fleet: { close: async () => {} },
    controller: { state: { phase: 'detail', executorMatch: true, outputs: ['out'], config: { computeId: 'worker-a' } }, choose() { this.state.phase = 'choose'; ui.render(); }, restoreContext() {}, close: async () => {} },
    discover: async () => { discoveries++; }, act: async (fn: any) => fn(),
    MutationObserver: class { constructor(callback: () => void) { observe = callback; } observe() {} },
  };
  runInNewContext(code, ui);
  return { ui, observe: () => observe(), counts: () => ({ reads, discoveries, lists }) };
}
test('actual preview activation coalesces pending operations and retains failures until explicit retry', async () => {
  const { ui } = harness(); let reads = 0, reject!: (error: Error) => void;
  ui.records.preview = () => { reads++; return new Promise((_resolve, r) => { reject = r; }); };
  ui.activateReads(); ui.activateReads(); assert.equal(reads, 1);
  reject(Error('denied')); await tick(); ui.activateReads(); assert.equal(reads, 1);
  assert.match(ui.get('jobs-output-preview').textContent, /unavailable/);
  ui.previewStates.delete('out'); ui.activateReads(); assert.equal(reads, 2); reject(Error('denied')); await tick();
});
test('actual hidden navigation sets dashboard before synchronous publish and starts no late output read', async () => {
  const { ui, observe, counts } = harness(); const rendered: string[] = [];
  ui.controller.state.phase = 'review'; ui.draftInput = 'old'; ui.section.hidden = true;
  ui.render = () => rendered.push(ui.override);
  observe(); assert.deepEqual(rendered, ['dashboard', 'dashboard']); assert.equal(ui.draftInput, '');
  ui.controller.state = { phase: 'detail', executorMatch: true, outputs: ['out'] }; ui.activateReads(); assert.equal(counts().reads, 0);
  ui.section.hidden = false; ui.entered = true; observe(); assert.equal(ui.override, 'dashboard');
});
test('actual list and preselected preview use independent generations; search cannot discard preview', async () => {
  const { ui } = harness(); let resolve!: (value: unknown) => void;
  ui.screen = 'choose'; ui.draftInput = 'input';
  ui.records.preview = () => new Promise(r => { resolve = r; });
  ui.activateReads(); await tick(); assert.match(ui.get('jobs-record-status').textContent, /No matching/);
  await ui.loadPicker('exact name'); resolve({ record: { name: 'Selected name' }, text: 'useful text' }); await tick();
  assert.equal(ui.get('jobs-input-preview').textContent, 'useful text');
});
test('actual navigation invalidation fences held preview completion', async () => {
  const { ui, observe } = harness(); let resolve!: (value: unknown) => void;
  ui.records.preview = () => new Promise(r => { resolve = r; }); ui.activateReads();
  ui.section.hidden = true; observe(); resolve({ record: { name: 'Old name' }, text: 'stale' }); await tick();
  assert.equal(ui.previews.size, 0); assert.notEqual(ui.get('jobs-output-preview').textContent, 'stale');
});
test('actual evidence key invalidates running-to-completed and changed executor claims', () => {
  const { ui } = harness(); const job = { status: 'running', updated_at: 'one' }, key = ui.evidence(job);
  assert.notEqual(key, ui.evidence({ ...job, status: 'completed', updated_at: 'two' }));
  ui.controller.state.config.computeId = 'worker-b'; assert.notEqual(key, ui.evidence(job));
});
test('actual refreshContext waits for prior close before exactly one visible ready discovery', async () => {
  const { ui, counts } = harness(); let release!: () => void;
  ui.controller.close = () => new Promise<void>(r => { release = r; });
  const next = { ...ui.context, domainId: 'b', data: {} }; ui.getContext = () => next;
  ui.refresh(); ui.enterVisible(); assert.equal(counts().discoveries, 0);
  release(); await tick(); ui.enterVisible(); assert.equal(counts().discoveries, 1);
});
test('render contains no picker or preview read triggers', () => {
  let body = ''; function find(node: ts.Node) { if (ts.isFunctionDeclaration(node) && node.name?.text === 'render') body = node.body!.getText(tree); ts.forEachChild(node, find); } find(tree);
  // Event handlers explicitly initiate reads; rendering itself must not invoke them.
  assert.ok(!body.includes('void loadPicker();')); assert.ok(!body.includes('else if (!busy) void previewRecord'));
});
