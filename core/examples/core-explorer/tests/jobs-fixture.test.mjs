import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createHash } from 'node:crypto';
import { DOMAIN, OTHER } from './fixture.mjs';
import { createJobsModel, CONFIG, INPUT, COST, CURSOR, capability } from './jobs-fixture.mjs';
const spec = role => ({ domain_id: DOMAIN, label: 'synthetic-reviewed-intent', priority: 0, meta: {}, edges: [], tasks: [{ label: role, stage: role, capability: capability(role), mode: 'dedicated', capability_filters: {}, priority: 0, inputs_cids: [INPUT], outputs_prefix: null, meta: { input_id: INPUT, run_id: CONFIG.installationId }, max_attempts: 1 }] });
function setup() {
  const model = createJobsModel(), storage = { records: [], contents: new Map([[INPUT, Buffer.from('Synthetic input\n')]]) };
  let token = model.grant(DOMAIN, 'http://127.0.0.1:9').access_token;
  const request = (method, path, body) => model.dispatch({ method, url: new URL(path, 'http://127.0.0.1:9'), headers: { authorization: 'Bearer ' + token, 'posemesh-client-id': 'synthetic-jobs-test', 'posemesh-sdk-version': 'auki-sdk/test' }, body }, storage);
  return { model, storage, request, token: value => { token = value; } };
}
for (const role of ['compute', 'robot']) test(`synthetic ${role} uses provider fields, exact costs and receipt identity`, () => {
  const f = setup(), body = spec(role);
  const estimate = f.request('POST', '/v1/jobs/estimate', body);
  assert.equal(estimate.status, 200); assert.equal(estimate.body.total, COST);
  assert.equal(estimate.body.tasks[0].estimated_credit_cost, COST);
  assert.equal(f.model.jobs.size, 0);
  const created = f.request('POST', '/v1/jobs', body); assert.equal(created.status, 200);
  const details = f.request('GET', '/v1/jobs/' + created.body.job_id).body;
  assert.equal(details.tasks[0].reserved_by, null);
  assert.equal(details.receipts[0].meta.status, undefined);
  assert.equal(details.receipts[0].meta.data_id, details.receipts[0].outputs[0]);
  assert.equal(details.receipts[0].node_id, CONFIG[`${role}Id`]);
  assert.equal(details.tasks[0].max_attempts, 1); assert.equal(details.tasks[0].mode, 'dedicated');
  const bytes = f.storage.contents.get(details.receipts[0].outputs[0]);
  const receiptBytes = role === 'compute' ? bytes : f.storage.contents.get(INPUT);
  assert.deepEqual(details.receipts[0].meta, { data_id: details.receipts[0].outputs[0], run_id: CONFIG.installationId, sha256: createHash('sha256').update(receiptBytes).digest('hex'), bytes: receiptBytes.length });
  assert.equal(f.storage.records[0].name, `sdk-${CONFIG.installationId}-${role}-${details.tasks[0].id}`);
  if (role === 'compute') assert.equal(bytes.toString(), 'SYNTHETIC INPUT\n');
  else { const report = JSON.parse(bytes); assert.equal(report.bytes, 16); assert.equal(report.input_id, INPUT); assert.match(report.sha256, /^[a-f0-9]{64}$/); }
  assert.deepEqual(f.model.state.violations, []);
});
test('all job routes enforce issued write grant, audience, expiry and Domain', () => {
  const f = setup(); const id = f.request('POST', '/v1/jobs', spec('compute')).body.job_id;
  for (const claims of [{ scopes: ['domain-data:r'] }, { aud: ['domain-manager'] }, { exp: 1 }, { iss: 'not-dds' }]) {
    f.token(f.model.grant(DOMAIN, 'http://127.0.0.1:9', claims).access_token);
    for (const [method, path, body] of [['POST', '/v1/jobs', spec('compute')], ['POST', '/v1/jobs/estimate', spec('compute')], ['GET', '/v1/jobs'], ['GET', '/v1/jobs/' + id], ['POST', `/v1/jobs/${id}/cancel`]]) assert.equal(f.request(method, path, body).status, 401);
  }
  f.token('unissued.synthetic.bearer'); assert.equal(f.request('GET', '/v1/jobs').status, 401);
  f.token(f.model.grant(OTHER, 'http://127.0.0.1:9').access_token);
  assert.equal(f.request('POST', '/v1/jobs', spec('compute')).status, 403);
  assert.equal(f.request('GET', '/v1/jobs/' + id).status, 404);
});
test('denial, no workers, ambiguous accepted submission, and cancellation are distinct', () => {
  const f = setup(); f.model.state.deny = true;
  assert.equal(f.request('POST', '/v1/jobs/estimate', spec('compute')).status, 403);
  f.model.state.deny = false; f.model.state.noWorkers = true;
  assert.equal(f.request('POST', '/v1/jobs/estimate', spec('compute')).status, 400); assert.equal(f.model.jobs.size, 0);
  f.model.state.noWorkers = false; f.model.state.uncertain = true; f.model.state.running = true;
  assert.equal(f.request('POST', '/v1/jobs', spec('compute')).body.job_id, 'ambiguous-synthetic-response');
  assert.equal(f.model.jobs.size, 1);
  const id = f.model.state.submissions[0].id;
  assert.equal(f.request('POST', `/v1/jobs/${id}/cancel`).body.status, 'canceled');
  const detail = f.request('GET', '/v1/jobs/' + id).body;
  assert.equal(detail.tasks[0].status, 'running'); assert.equal(detail.job.credit_released_at, null);
  assert.deepEqual(detail.receipts, []);
});
test('history passes opaque cursor unchanged; wrong routes and camelCase HTTP fail visibly', () => {
  const f = setup(); f.request('POST', '/v1/jobs', spec('compute'));
  const query = new URLSearchParams({ domain_id: DOMAIN, limit: '20', match_all_capabilities: 'false' });
  for (const role of ['compute', 'robot']) query.append('capabilities', capability(role));
  const page = f.request('GET', '/v1/jobs?' + query); assert.equal(page.status, 200); assert.equal(page.body.next_cursor, CURSOR);
  query.set('cursor', CURSOR); assert.deepEqual(f.request('GET', '/v1/jobs?' + query).body, { items: [], next_cursor: null });
  assert.equal(f.request('POST', '/v1/jobs/estimate/extra', spec('compute')).status, 422);
  const wrong = spec('compute'); wrong.tasks[0].maxAttempts = 1; delete wrong.tasks[0].max_attempts;
  assert.equal(f.request('POST', '/v1/jobs', wrong).status, 422);
  assert.equal(f.model.state.violations.length, 2);
});
test('executor mismatch and absent identity remain unverified synthetic receipts', () => {
  for (const mode of ['mismatch', 'missingExecutor']) {
    const f = setup(); f.model.state[mode] = true;
    const id = f.request('POST', '/v1/jobs', spec('robot')).body.job_id;
    const detail = f.request('GET', '/v1/jobs/' + id).body;
    assert.notEqual(detail.receipts[0].node_id, CONFIG.robotId);
    assert.notEqual(detail.tasks[0].reserved_by, CONFIG.robotId);
  }
});
