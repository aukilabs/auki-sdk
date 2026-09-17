import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import ts from 'typescript';
import { JobsController, type JobsContext } from '../src/jobs.ts';
import { FleetController } from '../src/fleet.ts';

// Run the real discovery and Back handlers, with real controllers and offline ports.
const source = readFileSync(new URL('../src/jobs-ui.ts', import.meta.url), 'utf8');
const tree = ts.createSourceFile('jobs-ui.ts', source, ts.ScriptTarget.Latest, true);
const selected: string[] = [];
function visit(node: ts.Node) {
  if (ts.isFunctionDeclaration(node) && ['discover', 'finishDiscovery', 'dashboard'].includes(node.name?.text ?? '')
    || ts.isExpressionStatement(node) && node.getText(tree).startsWith("button('jobs-back').onclick")) selected.push(node.getText(tree));
  ts.forEachChild(node, visit);
}
visit(tree);
const code = ts.transpileModule(selected.join('\n'), { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText;
const domain = 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa';
const computeId = 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb';
const robotId = 'cccccccc-cccc-4ccc-8ccc-cccccccccccc';
for (const mode of ['empty', 'failed', 'ambiguous']) test(`${mode} rediscovery followed by actual Back handler cannot estimate stale workers`, async () => {
  let outcome = 'success', estimates = 0;
  const session = {};
  const snapshot = (view: string) => {
    if (outcome === 'failed') throw Error('synthetic');
    const kind = view === 'domain' ? 'robot' : 'compute';
    const machine = { id: kind === 'robot' ? robotId : computeId, kind, association: kind === 'robot' ? 'assigned' : 'candidate', mode: 'dedicated', capabilities: [`/examples/compute-robot/${domain}/${kind}/v1`] };
    return { domain_id: domain, view, complete: true, machines: outcome === 'empty' ? [] : outcome === 'ambiguous' ? [machine, { ...machine, id: kind === 'robot' ? domain : 'dddddddd-dddd-4ddd-8ddd-dddddddddddd' }] : [machine], sources: [{ source: kind === 'robot' ? 'robots' : 'nodes', state: 'complete' }] } as any;
  };
  const fleet = new FleetController(() => ({ domainId: domain, session, environment: 'fixture', createFleet: () => ({ list: async () => snapshot('domain'), computePool: async () => snapshot('compute_pool'), close: async () => {}, free() {} }) }));
  const context = { domainId: domain, session, environment: 'fixture', data: { get: async () => ({ id: computeId, domain_id: domain, size: 0 }), readTo: async () => 0 }, createJobs: () => ({ estimate: async () => { estimates++; return { total: '0', tasks: [] }; }, list: async () => ({ items: [] }), close: async () => {} }) } as unknown as JobsContext;
  const controller = new JobsController(() => context), back = { onclick: () => {} };
  const ui: any = { controller, fleet, generation: 0, discoveryReady: false, ready: true, section: { hidden: false }, pickerVersion: 0, override: 'setup', screen: 'setup', draftRole: 'compute', invalidateReads() {}, draftInput: '', render() {}, button: () => back, options: { navigate() {} } };
  ui.act = async (action: () => Promise<void>) => { await action(); };
  runInNewContext(code, ui);
  await ui.discover(); await controller.prepare('compute', computeId); assert.equal(estimates, 1);
  outcome = mode; ui.screen = 'setup'; ui.override = 'setup'; await ui.discover(); back.onclick(); await new Promise(resolve => setImmediate(resolve));
  assert.equal(ui.override, 'dashboard'); assert.equal(controller.state.discoveryRequired, true);
  assert.equal(controller.state.spec, undefined); await controller.submit(); await controller.prepare('compute', computeId);
  assert.equal(estimates, 1); await controller.list(); assert.equal(controller.state.phase, 'history');
  assert.equal(controller.state.config?.computeId, computeId);
  await Promise.all([controller.close(), fleet.close()]);
});
