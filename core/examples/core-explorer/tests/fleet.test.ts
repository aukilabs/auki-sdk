import { test } from 'node:test';
import assert from 'node:assert/strict';
import { parseCapability, installations, FleetController } from '../src/fleet.ts';
import type { FleetMachine, FleetSnapshot } from '../../portable-echo/web/pkg-web/auki_portable_echo_web.js';
const domain = 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa';
const id = 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb';
const cap = (role: string) => `/examples/compute-robot/${id}/${role}/v1`;
const machine = (kind: 'compute' | 'robot', overrides = {}): FleetMachine => ({ kind, id, organization_id: domain, name: kind, capabilities: [cap(kind)], mode: 'dedicated', association: kind === 'robot' ? 'assigned' : 'candidate', presence: 'unknown', provider_status: 'new-status', presence_observed_at: '', last_seen_at: null, active_lease_expires_at: null, work_state: 'unknown', work_observed_at: null, activity: [], ...overrides });
const snapshot = (view: 'domain' | 'compute_pool', machines: FleetMachine[]): FleetSnapshot => ({ domain_id: domain, view, observed_at: '', machines, unresolved_activity: [], sources: [{ source: view === 'domain' ? 'robots' : 'nodes', state: 'complete', codes: [], observed_at: '', http_status: null }], complete: true });
test('exact canonical namespace only', () => {
  assert.deepEqual(parseCapability(cap('robot')), { installationId: id, role: 'robot' });
  for (const bad of [cap('robot').toUpperCase(), cap('robot') + '/', cap('robots'), cap('compute').replace('/v1', '/v2'), cap('robot').replace(id, 'bad')]) assert.equal(parseCapability(bad), undefined);
});
test('assigned robots and dedicated candidates stay separate; missing and ambiguous roles fail closed', () => {
  const robot = snapshot('domain', [machine('robot'), machine('compute', { id: domain, association: 'active_task' })]);
  const pool = snapshot('compute_pool', [machine('compute', { id: domain })]);
  let items = installations(domain, robot, pool);
  assert.equal(items.length, 1); assert.equal(items[0].config.robotId, id); assert.equal(items[0].config.computeId, domain);
  pool.machines.push(machine('compute', { id: 'cccccccc-cccc-4ccc-8ccc-cccccccccccc' }));
  items = installations(domain, robot, pool); assert.equal(items[0].config.computeId, undefined); assert.match(items[0].issues.join(' '), /ambiguous/i);
  assert.equal(installations(domain, robot)[0].config.computeId, undefined);
  assert.throws(() => installations(id, robot, pool), /Domain/);
  robot.machines.push(machine('robot')); assert.throws(() => installations(domain, robot), /Duplicate/);
});
test('partial sources retain discoveries and offline/unknown observations without claiming eligibility', () => {
  const pool = snapshot('compute_pool', [machine('compute', { presence: 'offline' })]);
  pool.complete = false; pool.sources.push({ source: 'busy', state: 'denied', codes: ['authorization_denied'], observed_at: '', http_status: 403 });
  assert.equal(installations(domain, undefined, pool)[0].compute[0].presence, 'offline');
  pool.sources[0].state = 'partial'; assert.equal(installations(domain, undefined, pool)[0].config.computeId, undefined);
});
test('close aborts, drains then frees; late Domain results are fenced', async () => {
  let resolve!: (s: FleetSnapshot) => void, signal: AbortSignal | null | undefined;
  const order: string[] = [];
  const pending = new Promise<FleetSnapshot>(r => { resolve = r; });
  let context = { domainId: domain, session: {}, environment: 'fixture', createFleet: () => ({ list: async (_q: unknown, s?: AbortSignal | null) => { signal = s; return pending; }, computePool: async () => snapshot('compute_pool', []), close: async () => { order.push('close'); }, free: () => { order.push('free'); } }) };
  const controller = new FleetController(() => context);
  const read = controller.refresh(); await Promise.resolve();
  context = { ...context, domainId: id }; const close = controller.close();
  assert.equal(signal?.aborted, true); assert.deepEqual(order, []);
  resolve(snapshot('domain', [machine('robot')])); await Promise.all([read, close]);
  assert.deepEqual(order, ['close', 'free']); assert.equal(controller.state.inventory, undefined);
});
test('cleanup failure is visible and blocks reuse', async () => {
  const session = {};
  const controller = new FleetController(() => ({ domainId: domain, environment: 'fixture', session, createFleet: () => ({ list: async () => snapshot('domain', []), computePool: async () => snapshot('compute_pool', []), close: async () => { throw Error('secret'); }, free: () => {} }) }));
  await controller.refresh(); await assert.rejects(controller.close(), /Fleet cleanup failed/); assert.match(controller.state.message, /cleanup failed/);
});
test('same-tick refresh then close prevents stale client creation and uses the next context', async () => {
  const created: string[] = [], closed: string[] = [], freed: string[] = [];
  let selected = domain;
  const session = {};
  const controller = new FleetController(() => {
    const selectedDomain = selected;
    return { domainId: selectedDomain, session, environment: 'fixture', createFleet: () => {
      created.push(selectedDomain);
      return { list: async () => ({ ...snapshot('domain', []), domain_id: selectedDomain }), computePool: async () => ({ ...snapshot('compute_pool', []), domain_id: selectedDomain }), close: async () => { closed.push(selectedDomain); }, free: () => { freed.push(selectedDomain); } };
    } };
  });
  const read = controller.refresh(), closing = controller.close();
  await Promise.all([read, closing]);
  assert.deepEqual(created, []); assert.deepEqual(closed, []); assert.deepEqual(freed, []);
  selected = id;
  await controller.refresh(); await controller.close();
  assert.deepEqual(created, [id]); assert.deepEqual(closed, [id]); assert.deepEqual(freed, [id]);
});
for (const role of ['compute', 'robot'] as const) test(`cross-kind exact ${role} capability collisions disable that action without losing provenance`, () => {
  const robot = machine('robot', { capabilities: [cap('robot'), cap(role)] });
  const compute = machine('compute', { id: domain, capabilities: [cap('compute'), cap(role)] });
  const [choice] = installations(domain, snapshot('domain', [robot]), snapshot('compute_pool', [compute]));
  assert.equal(choice.config[role === 'compute' ? 'computeId' : 'robotId'], undefined);
  assert.match(choice.issues.join(' '), new RegExp(`${role}: ambiguous executors \\(2\\)`));
  assert.deepEqual(choice.robot, [robot]); assert.deepEqual(choice.compute, [compute]);
});
test('denied opposite-kind inventory cannot establish a unique executor', () => {
  const robots = snapshot('domain', []); robots.sources[0].state = 'denied';
  const [choice] = installations(domain, robots, snapshot('compute_pool', [machine('compute')]));
  assert.equal(choice.config.computeId, undefined); assert.match(choice.issues.join(' '), /incomplete/);
});
