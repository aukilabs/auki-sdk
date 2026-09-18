import { test } from 'node:test';
import assert from 'node:assert/strict';
import { fleetResponse } from './fleet-fixture.mjs';
import { CONFIG, capability } from './jobs-fixture.mjs';
import { DOMAIN, OTHER } from './fixture.mjs';
const read = (path, state = {}, headers = { authorization: 'Bearer synthetic-service' }) => fleetResponse({ method: 'GET', url: new URL(path, 'http://127.0.0.1'), headers }, state, { domain: DOMAIN, other: OTHER, config: CONFIG, capability });
test('DDS wire robots are assigned; dedicated nodes use broad inventory query', () => {
  const robots = read(`/api/v1/domains/${DOMAIN}/robots`);
  assert.equal(robots.body.robots[0].assigned_domain_id, DOMAIN);
  assert.equal(robots.body.robots[0].capabilities[0], capability('robot'));
  const nodes = read('/api/v1/nodes?org=all&staking_status=all');
  assert.equal(nodes.body.nodes[0].mode, 'dedicated'); assert.equal(nodes.body.nodes[0].assigned_domain_id, undefined);
  assert.equal(read('/api/v1/nodes').status, 422);
  assert.equal(read(`/api/v1/domains/${DOMAIN}/robots`, {}, {}).status, 403);
});
test('denial, empty, conflicting Domain and duplicate candidates remain distinct provider outcomes', () => {
  const path = `/api/v1/domains/${DOMAIN}/robots`;
  assert.equal(read(path, { fleetDenied: 'robots' }).status, 403);
  assert.deepEqual(read(path, { fleetEmpty: true }).body, { robots: [] });
  assert.equal(read(path, { fleetWrongDomain: true }).body.robots[0].assigned_domain_id, OTHER);
  assert.equal(read('/api/v1/nodes?org=all&staking_status=all', { fleetDuplicate: true }).body.nodes.length, 2);
});
for (const role of ['compute', 'robot']) test(`provider fixture exposes cross-kind exact ${role} capability collisions`, () => {
  const state = { fleetCrossKind: role };
  const robot = read(`/api/v1/domains/${DOMAIN}/robots`, state).body.robots[0];
  const compute = read('/api/v1/nodes?org=all&staking_status=all', state).body.nodes[0];
  assert.ok(robot.capabilities.includes(capability(role)));
  assert.ok(compute.capabilities.includes(capability(role)));
  assert.notEqual(robot.id, compute.id);
  assert.equal(robot.assigned_domain_id, DOMAIN); assert.equal(compute.mode, 'dedicated');
});
