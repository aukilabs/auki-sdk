import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import ts from 'typescript';
import { JobsController, HISTORY_WARNING, type JobsContext } from '../src/jobs.ts';
import { FleetController } from '../src/fleet.ts';

// Execute production handlers/render/act with real controllers; only DOM and service ports are offline doubles.
const source = readFileSync(new URL('../src/jobs-ui.ts', import.meta.url), 'utf8');
const tree = ts.createSourceFile('jobs-ui.ts', source, ts.ScriptTarget.Latest, true);
const pieces: string[] = [];
function visit(n: ts.Node) {
  if (ts.isFunctionDeclaration(n) && ['render', 'changeInput', 'evidence', 'selectedInput', 'enterVisible', 'discover', 'finishDiscovery', 'dashboard', 'act', 'close', 'edit'].includes(n.name?.text ?? '')) pieces.push(n.getText(tree));
  if (ts.isExpressionStatement(n) && (n.getText(tree).startsWith('new MutationObserver') || /^button\('jobs-(history|back)'\).onclick/.test(n.getText(tree)))) pieces.push(n.getText(tree));
  ts.forEachChild(n, visit);
}
visit(tree);
const code = ts.transpileModule(pieces.join('\n'), { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText;
const tick = () => new Promise(resolve => setImmediate(resolve));
const domain = 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa', worker = 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb', input = 'cccccccc-cccc-4ccc-8ccc-cccccccccccc';
function deferred<T>() { let resolve!: (value: T) => void; const promise = new Promise<T>(r => { resolve = r; }); return { promise, resolve }; }
function harness(choices = 1) {
  const nodes = new Map<string, any>();
  const node = (): any => ({ dataset: {}, parentElement: { hidden: false }, hidden: false, disabled: false, open: false, children: [], append(...items: any[]) { this.children.push(...items); }, prepend() {}, replaceChildren(...items: any[]) { this.children = items; }, insertAdjacentHTML() {}, focus() { this.focused = true; }, setAttribute() {}, querySelector: node, querySelectorAll: () => [] });
  const get = (id: string) => { if (!nodes.has(id)) nodes.set(id, node()); return nodes.get(id); };
  let observer!: () => void, reads = 0, discoveries = 0, submissions = 0;
  const discovery = deferred<void>(), submission = deferred<void>();
  const ui: any = { historyPage: undefined, restoreHistory: false, ready: true, entered: true, busy: false, generation: 0, discoveryReady: false, screen: 'dashboard', override: 'dashboard', draftInput: '', draftRole: 'compute', polls: 0, closing: Promise.resolve(),
    section: { dataset: {}, scrollTop: 0, hidden: false, contains: () => false, querySelector: (selector: string) => selector === 'details' ? get('details') : get('jobs-history-items').children?.findLast((child: any) => '#' + child.id === selector) ?? get(selector), querySelectorAll: () => [] },
    document: { activeElement: null, createElement: node }, get, button: get, field: get, text(id: string, value: unknown) { get(id).textContent = value; }, redact: (x: unknown) => x, inspect: JSON.stringify, facts: node,
    names: new Map(), previews: new Map(), summaries: new Map(), previewStates: new Map(), options: {},
    invalidateReads() {}, stopPolling() {}, activateReads() {}, schedule() {}, records: { close: async () => {} },
    MutationObserver: class { constructor(fn: () => void) { observer = fn; } observe() {} }, HISTORY_WARNING,
  };
  const context = { domainId: domain, environment: 'fixture', session: {}, data: { get: async () => ({ id: input, domain_id: domain, size: 0 }), readTo: async () => 0 },
    createJobs: () => ({ estimate: async () => ({ total: '0.123456789012345678', tasks: [] }), submit: async () => { submissions++; await submission.promise; throw { code: 'submission_uncertain' }; }, list: async () => { reads++; assert.equal(ui.section.hidden, false, 'history must only start while visible'); return { items: [] }; }, close: async () => {} }),
  } as unknown as JobsContext;
  ui.context = context; ui.getContext = () => ui.context; ui.connection = { session: context.session };
  const snapshot = (view: string) => ({ domain_id: domain, view, complete: true, sources: [{ source: view === 'domain' ? 'robots' : 'nodes', state: 'complete' }], machines: view === 'domain' ? [] : Array.from({ length: choices }, (_, i) => ({ id: i ? input : worker, kind: 'compute', association: 'candidate', mode: 'dedicated', presence: 'online', work_state: 'unknown', capabilities: [`/examples/compute-robot/${i ? input : domain}/compute/v1`] })) }) as any;
  ui.controller = new JobsController(() => ui.context, () => ui.render());
  ui.fleet = new FleetController(() => ({ ...ui.context, createFleet: () => ({ list: async () => { discoveries++; await discovery.promise; return snapshot('domain'); }, computePool: async () => snapshot('compute_pool'), close: async () => {}, free() {} }) }));
  runInNewContext(code, ui);
  return { ui, get, discovery, submission, counts: () => ({ reads, discoveries, submissions }), visible(value: boolean) { ui.section.hidden = !value; observer(); }, close: () => Promise.all([ui.controller.close(), ui.fleet.close()]) };
}
for (const offscreen of [false, true]) test(`uncertainty retains usable reconciliation after navigation; settles offscreen=${offscreen}`, async () => {
  const h = harness(), { ui } = h;
  await ui.controller.configure({ installationId: domain, computeId: worker });
  await ui.controller.prepare('compute', input); ui.override = undefined;
  const pending = ui.act(() => ui.controller.submit());
  if (offscreen) h.visible(false);
  h.submission.resolve(); await pending;
  assert.equal(ui.controller.state.phase, 'uncertain');
  if (!offscreen) h.visible(false);
  h.visible(true);
  assert.equal(h.get('jobs-history').hidden, false, 'reconciliation route must remain visible even on empty dashboard');
  assert.equal(h.get('jobs-history').disabled, false);
  assert.equal(h.get('jobs-new').hidden, true);
  h.get('jobs-history').onclick(); await tick();
  assert.equal(h.counts().reads, 1);
  assert.ok(ui.controller.state.reconciliation);
  assert.equal(h.get('jobs-history').hidden, false, 'empty history must keep a retry/reconciliation route');
  await ui.controller.submit(); assert.equal(h.counts().submissions, 1);
  await h.close();
});
for (const timing of ['after', 'before', 'before_then_hidden']) test(`held discovery resumes once on visible return; timing=${timing}`, async () => {
  const h = harness(), { ui } = h;
  ui.entered = false; ui.enterVisible(); await tick();
  h.visible(false);
  if (timing !== 'after') { h.visible(true); h.visible(false); h.visible(true); }
  if (timing === 'before_then_hidden') h.visible(false);
  h.discovery.resolve(); await tick();
  assert.equal(h.counts().reads, timing === 'before' ? 1 : 0, 'no hidden history continuation');
  if (timing !== 'before') { h.visible(true); await tick(); }
  assert.equal(h.counts().reads, 1, 'visible return activates exactly one list');
  h.visible(false); h.visible(true); await tick();
  assert.deepEqual(h.counts(), { reads: 1, discoveries: 1, submissions: 0 });
  assert.equal(ui.controller.state.config.computeId, worker);
  await h.close();
});
test('hidden multi-installation discovery preserves explicit choice without auto history', async () => {
  const h = harness(2); h.ui.entered = false; h.ui.enterVisible(); await tick();
  h.visible(false); h.discovery.resolve(); await tick(); h.visible(true); await tick();
  assert.equal(h.ui.fleet.state.installations.length, 2);
  assert.equal(h.ui.controller.state.config, undefined);
  assert.deepEqual(h.counts(), { reads: 0, discoveries: 1, submissions: 0 });
  await h.close();
});
test('close during held discovery fences stale activation and drains the real controllers', async () => {
  const h = harness(); h.ui.entered = false; h.ui.enterVisible(); await tick();
  h.visible(false); const closing = h.ui.close(); h.discovery.resolve(); await closing; await tick();
  h.visible(true); await tick();
  assert.equal(h.ui.controller.state.config, undefined);
  assert.equal(h.counts().reads, 0); assert.equal(h.ui.ready, false);
  await h.close();
});

test('close discards discovery already settled while hidden when the Domain context changes', async () => {
  const h = harness(); h.ui.entered = false; h.ui.enterVisible(); await tick();
  h.visible(false); h.discovery.resolve(); await tick();
  assert.equal(h.ui.discoveryReady, true);
  const next = { ...h.ui.context, domainId: input };
  h.ui.getContext = () => next;
  await h.ui.close();
  assert.equal(h.ui.discoveryReady, false);
  h.visible(true); await tick();
  assert.equal(h.ui.controller.state.config, undefined);
  assert.equal(h.counts().reads, 0);
  await h.close();
});

for (const route of ['settings', 'technical']) test(`actual auxiliary ${route} return keeps input and role but invalidates spending review`, async () => {
  const h = harness(), { ui } = h;
  await ui.controller.configure({ installationId: domain, computeId: worker });
  ui.override = undefined;
  await ui.controller.prepare('compute', input);
  assert.equal(ui.controller.state.phase, 'review');
  assert.equal(ui.controller.state.estimate.total, '0.123456789012345678');
  ui.previews.set(input, 'Retained useful preview');
  ui.section.dataset.auxiliary = 'true';
  h.visible(false);
  assert.equal(ui.draftInput, input);
  assert.equal(ui.draftRole, 'compute');
  assert.equal(ui.controller.state.estimate, undefined);
  assert.equal(ui.controller.state.spec, undefined);
  h.visible(true);
  assert.equal(ui.screen, 'choose');
  assert.equal(ui.previews.get(input), 'Retained useful preview');
  assert.equal(h.counts().submissions, 0);
  await ui.controller.submit();
  assert.equal(h.counts().submissions, 0, 'old review cannot submit on return');
  await h.close();
});

test('actual history row and Back handlers restore cached page, selected row and scroll without provider read', async () => {
  const h = harness(), { ui } = h;
  let lists = 0;
  const original = ui.context.createJobs;
  ui.context.createJobs = () => ({ ...original(), list: async () => {
    lists++; return { items: [{ job: { id: input, domain_id: domain, status: 'completed' } }], next_cursor: 'opaque-page-two' };
  }, get: async () => { throw Error('offline detail unavailable'); } });
  await ui.controller.configure({ installationId: domain, computeId: worker });
  await ui.controller.list('opaque-current-page');
  ui.section.scrollTop = 214;
  h.get('jobs-history-items').children.at(-1).onclick();
  await tick();
  assert.equal(ui.screen, 'detail');
  h.get('jobs-back').onclick(); await tick();
  assert.equal(ui.screen, 'dashboard');
  assert.equal(lists, 1, 'Back uses the unchanged cached page');
  ui.section.dataset.auxiliary = 'true'; h.visible(false); h.visible(true);
  assert.equal(ui.screen, 'dashboard', 'cached history stays history across Settings');
  assert.equal(lists, 1);
  assert.equal(ui.section.scrollTop, 214);
  assert.ok(h.get('jobs-history-items').children.at(-1).focused);
  assert.equal(h.get('jobs-next').disabled, false);
  h.get('jobs-refresh-history').onclick(); await tick();
  assert.equal(lists, 2, 'explicit Refresh reaches the provider');
  await h.close();
});

test('actual review shows known Domain identity and exact decimal cost outside Details', async () => {
  const h = harness(), { ui } = h;
  const rendered: Record<string, unknown>[] = [];
  ui.facts = (value: Record<string, unknown>) => { rendered.push(value); return {}; };
  ui.options.domainName = () => 'Known Domain';
  await ui.controller.configure({ installationId: domain, computeId: worker });
  ui.override = undefined;
  await ui.controller.prepare('compute', input);
  const review = rendered.find(value => 'Estimated credits' in value);
  assert.equal(review?.Domain, domain);
  assert.equal(review?.Environment, 'fixture');
  assert.equal(review?.['Estimated credits'], '0.123456789012345678');
  await h.close();
});
