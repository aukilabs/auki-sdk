import test from 'node:test';
import assert from 'node:assert/strict';
import { aggregateFleet, FleetScreenController } from '../src/fleet-screen.ts';
import type { FleetMachine, FleetSnapshot } from '../../portable-echo/web/pkg-web/auki_portable_echo_web.js';
const machine = (id: string, mode = 'dedicated', extra = {}): FleetMachine => ({ id, mode, kind: 'compute', organization_id: 'org', name: 'Same name', capabilities: ['vendor/v1'], association: 'candidate', presence: 'online', provider_status: 'online', presence_observed_at: '', last_seen_at: null, active_lease_expires_at: null, work_state: 'idle', work_observed_at: '', activity: [], ...extra });
const snapshot = (view: FleetSnapshot['view'], machines: FleetMachine[], extra = {}): FleetSnapshot => ({ domain_id: 'domain', view, machines, observed_at: '', complete: true, unresolved_activity: [], sources: [], ...extra });
test('All joins UUIDs and preserves Domain observations while filtering provider mode', () => {
  const domain = snapshot('domain', [machine('robot', 'dedicated', { kind: 'robot', association: 'assigned' }), machine('node', 'dedicated', { association: 'active_task', work_state: 'unknown' }), machine('unknown', 'future')]);
  const pool = snapshot('compute_pool', [machine('node'), machine('public', 'public')]);
  const all = aggregateFleet('domain', [domain, pool]);
  assert.equal(all.machines.length, 4);
  assert.equal(all.machines.find(m => m.id === 'node')?.work_state, 'unknown');
  assert.equal(all.machines.find(m => m.id === 'node')?.association, 'active_task');
  assert.equal(all.machines.find(m => m.id === 'public')?.association, 'candidate');
  assert.equal(aggregateFleet('domain', [domain, pool], 'dedicated').machines.length, 2);
  assert.equal(aggregateFleet('domain', [domain, pool], 'public').machines.length, 1);
  assert.throws(() => aggregateFleet('wrong', [domain]), /Domain/);
  assert.throws(() => aggregateFleet('domain', [snapshot('domain', [machine('x'), machine('x')])]), /Duplicate/);
});
test('source denial and unresolved activity cannot become an empty success', () => {
  const result = aggregateFleet('domain', [snapshot('domain', [], { complete: false, sources: [{ source: 'robots', state: 'denied' }], unresolved_activity: [{ task_id: 'unresolved' }] })]);
  assert.equal(result.complete, false); assert.equal(result.unresolved.length, 1); assert.equal(result.sources[0].state, 'denied');
});
test('close fences deferred creation and aborts/drains before freeing', async () => {
  const events: string[] = []; let release!: () => void; let signal: AbortSignal | null | undefined;
  const held = new Promise<void>(r => { release = r; }); const session = {};
  const controller = new FleetScreenController(() => ({ domainId: 'domain', session, environment: 'local', createFleet: () => { events.push('create'); return { list: async (_q: unknown, s?: AbortSignal | null) => { signal = s; await held; return snapshot('domain', []); }, computePool: async () => snapshot('compute_pool', []), close: async () => { events.push('close'); }, free: () => { events.push('free'); } }; } }));
  const first = controller.refresh(); await controller.close(); await first; assert.deepEqual(events, []);
  const read = controller.refresh(); await new Promise(r => setImmediate(r));
  const close = controller.close(); assert.equal(signal?.aborted, true); assert.ok(!events.includes('free'));
  release(); await Promise.all([read, close]); assert.deepEqual(events, ['create', 'close', 'free']); assert.equal(controller.state.snapshots.length, 0);
});
test('refresh uses exactly two explicit pool modes and retains partial read failures for retry', async () => {
  let denied = true; const modes: string[] = [], session = {};
  const controller = new FleetScreenController(() => ({ domainId: 'domain', session, environment: 'local', createFleet: () => ({
    list: async () => { if (denied) throw Error('denied'); return snapshot('domain', [machine('robot', 'dedicated', { kind: 'robot', association: 'assigned' })]); },
    computePool: async (query: { mode: string }) => { modes.push(query.mode); return snapshot('compute_pool', [machine(query.mode, query.mode)]); },
    close: async () => {}, free: () => {},
  }) }));
  await controller.refresh(); assert.deepEqual(modes, ['dedicated', 'public']);
  assert.equal(controller.state.failures.length, 1); assert.match(controller.state.message, /Partial/);
  denied = false; await controller.refresh(); assert.equal(controller.state.failures.length, 0);
  assert.equal(aggregateFleet('domain', controller.state.snapshots).machines.length, 3);
  await controller.close();
});
test('concurrent refresh drains the aborted generation and publishes only the latest request', async () => {
  let created = 0; const session = {}, events: string[] = [];
  const controller = new FleetScreenController(() => ({ domainId: 'domain', session, environment: 'local', createFleet: () => {
    const generation = ++created;
    return { list: async (_q: unknown, signal?: AbortSignal | null) => {
      if (generation === 1) await new Promise<void>(resolve => signal?.addEventListener('abort', () => { events.push('aborted'); resolve(); }, { once: true }));
      return snapshot('domain', [machine(String(generation))]);
    }, computePool: async () => snapshot('compute_pool', []), close: async () => { events.push(`close${generation}`); }, free: () => { events.push(`free${generation}`); } };
  } }));
  const first = controller.refresh(); await new Promise(r => setImmediate(r));
  const second = controller.refresh(); await Promise.all([first, second]);
  assert.deepEqual(events, ['aborted', 'close1', 'free1', 'close2', 'free2']);
  assert.equal(controller.state.snapshots[0].machines[0].id, '2');
});
test('disagreeing busy/idle observations become Unknown and preserve authorized activity', () => {
  const work = { job_id: 'job', task_id: 'task' };
  const result = aggregateFleet('domain', [snapshot('domain', [machine('node', 'dedicated', { association: 'active_task', work_state: 'busy', activity: [work] })]), snapshot('compute_pool', [machine('node')])]);
  assert.equal(result.machines[0].work_state, 'unknown'); assert.deepEqual(result.machines[0].activity, [work]);
});

test('Fleet entry loads once, reuses its snapshot and loads again after a Domain reset', async () => {
  let created = 0; const session = {};
  const controller = new FleetScreenController(() => ({ domainId: 'domain', session, environment: 'local', createFleet: () => {
    created++;
    return { list: async () => snapshot('domain', [machine('node')]), computePool: async () => snapshot('compute_pool', []), close: async () => {}, free: () => {} };
  } }));
  const first = controller.enter(); assert.equal(controller.state.loading, true);
  assert.equal(first, controller.enter()); await first;
  assert.equal(created, 1); assert.equal(controller.state.snapshots[0].machines.length, 1);
  await controller.close(false); await controller.enter(); assert.equal(created, 1);
  await controller.close(); await controller.enter(); assert.equal(created, 2);
  await controller.close();
});

test('returning after an interrupted first Fleet read retries after cleanup', async () => {
  let created = 0; const session = {}, events: string[] = [];
  const controller = new FleetScreenController(() => ({ domainId: 'domain', session, environment: 'local', createFleet: () => {
    const attempt = ++created;
    return { list: async (_query: unknown, signal?: AbortSignal | null) => {
      if (attempt === 1) await new Promise<void>(resolve => signal?.addEventListener('abort', () => resolve(), { once: true }));
      return snapshot('domain', [machine(String(attempt))]);
    }, computePool: async () => snapshot('compute_pool', []), close: async () => { events.push(`close${attempt}`); }, free: () => { events.push(`free${attempt}`); } };
  } }));
  const first = controller.enter(); await new Promise(r => setImmediate(r));
  const leaving = controller.close(false), returning = controller.enter();
  await Promise.all([first, leaving, returning]);
  assert.equal(created, 2); assert.deepEqual(events, ['close1', 'free1', 'close2', 'free2']);
  assert.equal(controller.state.snapshots[0].machines[0].id, '2'); await controller.close();
});
