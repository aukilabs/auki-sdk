import test from 'node:test';
import assert from 'node:assert/strict';
import type { FleetMachine, FleetSnapshot } from '../../portable-echo/web/pkg-web/auki_portable_echo_web.js';
import { fleetAssociation, fleetCapability, fleetObservedAt, fleetOverview, fleetSourceNote, fleetTime } from '../src/fleet-presentation.ts';

test('overview counts presence separately from work and retains unknown observations', () => {
  const machines = [
    { kind: 'robot', presence: 'online', work_state: 'busy' },
    { kind: 'compute', presence: 'online', work_state: 'unknown' },
    { kind: 'compute', presence: 'offline', work_state: 'unknown' },
    { kind: 'compute', presence: 'unknown', work_state: 'unknown' },
  ] as FleetMachine[];
  assert.deepEqual(fleetOverview(machines), { robots: 1, compute: 3, online: 2, offline: 1, presenceUnknown: 1, busy: 1, idle: 0, workUnknown: 3 });
});

test('capability labels recognize only exact demo contracts and identify simulated inspection', () => {
  const prefix = '/examples/compute-robot/11111111-1111-4111-8111-111111111111';
  assert.equal(fleetCapability(`${prefix}/compute/v1`).name, 'Uppercase text');
  assert.match(fleetCapability(`${prefix}/robot/v1`).description, /Simulated/);
  for (const capability of ['vendor/arbitrary/v9', `${prefix}/robot/v2`, `${prefix}/robot/v1/extra`, `${prefix}/Robot/v1`, '<img src=x onerror=alert(1)>']) {
    assert.equal(fleetCapability(capability).name, capability, 'unknown capabilities keep their original identity');
  }
});

test('candidate association does not imply Domain assignment', () => {
  assert.equal(fleetAssociation({ association: 'candidate' } as FleetMachine), 'Visible compute candidate');
  assert.equal(fleetAssociation({ association: 'active_task' } as FleetMachine), 'Observed task in this Domain');
  assert.equal(fleetAssociation({ association: 'assigned' } as FleetMachine), 'Assigned to this Domain');
});

test('snapshot time uses the oldest valid read and does not invent missing timestamps', () => {
  const snapshots = ['2026-09-18T02:10:00Z', '2026-09-18T02:00:00Z', 'invalid'].map(observed_at => ({ observed_at }) as FleetSnapshot);
  assert.equal(fleetObservedAt(snapshots), '2026-09-18T02:00:00.000Z');
  assert.equal(fleetObservedAt([]), undefined);
  for (const time of [null, undefined, '', 'invalid']) assert.equal(fleetTime(time), 'Not reported');
});

test('partial sources and unresolved activity are explained without treating them as empty success', () => {
  const partial = [{ sources: [{ source: 'busy', state: 'denied' }, { source: 'busy', state: 'unsupported' }], unresolved_activity: [] }] as unknown as FleetSnapshot[];
  assert.match(fleetSourceNote(partial, []), /work status/);
  assert.equal(fleetSourceNote(partial, []).match(/work status/g)?.length, 1);
  assert.match(fleetSourceNote([], ['Read failed']), /could not be read/);
  assert.match(fleetSourceNote([{ sources: [], unresolved_activity: [{}] }] as unknown as FleetSnapshot[], []), /could not be matched/);
});
