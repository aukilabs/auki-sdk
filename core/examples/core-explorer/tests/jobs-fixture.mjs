// SYNTHETIC HTTP PROVIDER ONLY. No worker execution or production SDK replacement.
// Wire sources: auki-dms/{src/jobs,tests/jobs_contract.rs}, Web src/jobs.rs,
// docs/reference/jobs.md; provider http/routes/jobs.rs and http/auth/app.rs,
// DDS pkg/authpkg/domain_token.go. Browser callers use the real built WASM SDK.
import assert from 'node:assert/strict';
import { createHash, generateKeyPairSync, randomUUID, sign, verify } from 'node:crypto';
import { startFixture, DOMAIN, OTHER } from './fixture.mjs';
export const CONFIG = { installationId: '11111111-1111-4111-8111-111111111111', computeId: '22222222-2222-4222-8222-222222222222', robotId: '33333333-3333-4333-8333-333333333333' };
export const INPUT = '44444444-4444-4444-8444-444444444444';
export const COST = '12345678901234567890.123456789';
export const CURSOR = 'synthetic+/cursor==&keep-opaque';
export const capability = role => `/examples/compute-robot/${CONFIG.installationId}/${role}/v1`;
const timestamp = '2026-09-17T00:00:00Z';
const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;
const summary = status => ({ queued: 0, leased: 0, running: 0, completed: 0, failed: 0, canceled: 0, [status]: 1 });
const encode = value => Buffer.from(JSON.stringify(value)).toString('base64url');
export function assertJobSpec(body) {
  assert.deepEqual(Object.keys(body).sort(), ['domain_id', 'edges', 'label', 'meta', 'priority', 'tasks']);
  assert.ok([DOMAIN, OTHER].includes(body.domain_id));
  assert.ok(typeof body.label === 'string' && body.label.length > 0);
  assert.deepEqual(body.edges, []); assert.equal(body.tasks.length, 1);
  const task = body.tasks[0];
  assert.deepEqual(Object.keys(task).sort(), ['capability', 'capability_filters', 'inputs_cids', 'label', 'max_attempts', 'meta', 'mode', 'outputs_prefix', 'priority', 'stage']);
  assert.ok(['compute', 'robot'].some(role => capability(role) === task.capability));
  assert.equal(task.mode, 'dedicated'); assert.equal(task.max_attempts, 1);
  assert.equal(typeof task.label, 'string'); assert.equal(typeof task.stage, 'string');
  assert.ok(uuid.test(task.meta.input_id));
  assert.deepEqual(task.inputs_cids, [task.meta.input_id]);
  assert.equal(task.meta.run_id, CONFIG.installationId);
  assert.deepEqual(task.capability_filters, {});
  assert.equal(task.outputs_prefix, null);
  return task.capability === capability('compute') ? 'compute' : 'robot';
}
// Pure model: unit tests do not bind ports or launch any process.
export function createJobsModel() {
  const keys = generateKeyPairSync('ec', { namedCurve: 'prime256v1' });
  const issued = new Map(), jobs = new Map();
  const state = { requests: [], violations: [], grants: [], estimates: [], submissions: [], lists: [], cancellations: [], deny: false, readOnly: false, noWorkers: false, uncertain: false, mismatch: false, missingExecutor: false, running: false, hold: undefined };
  const grant = (domain, dataBase, overrides = {}) => {
    const now = Math.floor(Date.now() / 1000);
    const claims = { iss: 'dds', aud: ['dds', dataBase], sub: 'synthetic-user', org: CONFIG.installationId, type: 'user-access', domain_id: domain, scopes: [state.readOnly ? 'domain-data:r' : 'domain-data:rw'], iat: now, exp: now + 3600, ...overrides };
    const input = encode({ alg: 'ES256', typ: 'JWT' }) + '.' + encode(claims);
    const token = input + '.' + sign('sha256', Buffer.from(input), { key: keys.privateKey, dsaEncoding: 'ieee-p1363' }).toString('base64url');
    issued.set(token, claims); state.grants.push({ domain, scopes: claims.scopes });
    return { id: domain, domain_server: { url: dataBase }, access_token: token };
  };
  const dispatch = ({ method, url, headers = {}, body }, storage = { records: [], contents: new Map() }) => {
    const path = url.pathname, observation = { method, path, query: [...url.searchParams], body: structuredClone(body) };
    state.requests.push(observation);
    const response = (status, body) => ({ status, body });
    const token = headers.authorization?.replace(/^Bearer /, ''), claims = issued.get(token);
    const parts = token?.split('.');
    if (!claims || !verify('sha256', Buffer.from(parts.slice(0, 2).join('.')), { key: keys.publicKey, dsaEncoding: 'ieee-p1363' }, Buffer.from(parts[2], 'base64url')) || claims.iss !== 'dds' || !claims.aud.includes('dds') || !['user-access', 'app-access'].includes(claims.type) || !uuid.test(claims.org) || !uuid.test(claims.domain_id) || !claims.sub || claims.exp <= Date.now() / 1000 || !claims.scopes.some(s => ['domain:rw', 'domain-data:rw'].includes(s))) return response(401, {});
    if (state.deny) return response(403, { error: 'synthetic-backend-secret' });
    try {
      assert.ok(headers['posemesh-client-id']); assert.match(headers['posemesh-sdk-version'] ?? '', /^auki-sdk\//);
      if (method === 'POST' && ['/v1/jobs', '/v1/jobs/estimate'].includes(path)) {
        assert.equal(url.search, '');
        const role = assertJobSpec(body);
        if (body.domain_id !== claims.domain_id) return response(403, {});
        if (state.noWorkers) return response(400, { error: 'no available nodes for capabilities' });
        if (path.endsWith('/estimate')) {
          state.estimates.push(structuredClone(body));
          return response(200, { total: COST, tasks: body.tasks.map(t => ({ label: t.label, stage: t.stage, capability: t.capability, mode: t.mode, billing_units: '1.00', estimated_credit_cost: COST })) });
        }
        const id = randomUUID(), taskId = randomUUID(), output = randomUUID();
        const executor = state.missingExecutor ? null : state.mismatch ? OTHER : CONFIG[`${role}Id`];
        const status = state.running ? 'running' : 'completed';
        const source = Buffer.from(storage.contents.get(body.tasks[0].meta.input_id) ?? 'Synthetic input');
        const bytes = role === 'compute' ? Buffer.from(source.toString('utf8').toUpperCase()) : Buffer.from(JSON.stringify({ bytes: source.length, input_id: body.tasks[0].meta.input_id, sha256: createHash('sha256').update(source).digest('hex') }).replaceAll(':', ': ').replaceAll(',', ', '));
        storage.records.push({ id: output, domain_id: body.domain_id, name: `sdk-${CONFIG.installationId}-${role}-${taskId}`, data_type: role === 'compute' ? 'example.text.v1' : 'example.report.v1', size: bytes.length, created_at: timestamp, updated_at: timestamp }); storage.contents.set(output, bytes);
        const job = { id, domain_id: body.domain_id, label: body.label, priority: body.priority, meta: { ...body.meta, synthetic: true }, status, organization_id: CONFIG.installationId, created_at: timestamp, updated_at: timestamp, credit_lock_id: null, credit_lock_amount: COST, credit_locked_at: timestamp, credit_released_at: null };
        const task = { ...body.tasks[0], label: body.tasks[0].stage, id: taskId, job_id: id, status, deps_remaining: 0, organization_id: CONFIG.installationId, attempts: 1, reserved_by: state.running ? executor : null, lease_expires_at: null, meta: { ...body.tasks[0].meta, progress: { phase: status }, events: [{ phase: 'synthetic fixture result' }] }, cancel_requested_at: null, last_heartbeat_at: timestamp, created_at: timestamp, updated_at: timestamp, billing_units: '1.00', estimated_credit_cost: COST, debited_amount: null, debited_at: null };
        jobs.set(id, { job, tasks_summary: summary(status), tasks: [task], receipts: state.running ? [] : [{ id: randomUUID(), job_id: id, task_id: taskId, node_id: executor, outputs: [output], meta: { data_id: output, run_id: CONFIG.installationId, sha256: createHash('sha256').update(role === 'compute' ? bytes : source).digest('hex'), bytes: role === 'compute' ? bytes.length : source.length }, created_at: timestamp }] });
        state.submissions.push({ id, output, body: structuredClone(body) });
        // Invalid success after storage yields SDK submission_uncertain, without a
        // dropped socket that Chromium may transparently retransmit.
        return response(200, state.uncertain ? { job_id: 'ambiguous-synthetic-response' } : { job_id: id });
      }
      if (method === 'GET' && path === '/v1/jobs') {
        const q = url.searchParams;
        assert.equal(q.get('domain_id'), claims.domain_id);
        assert.ok(Number(q.get('limit')) >= 1 && Number(q.get('limit')) <= 100);
        assert.deepEqual(q.getAll('capabilities').sort(), [capability('compute'), capability('robot')].sort());
        assert.equal(q.get('match_all_capabilities'), 'false');
        assert.ok([...q.keys()].every(k => ['domain_id', 'limit', 'capabilities', 'match_all_capabilities', 'cursor'].includes(k)));
        assert.ok(!q.has('cursor') || q.get('cursor') === CURSOR);
        state.lists.push([...q]);
        const items = [...jobs.values()].reverse().filter(d => d.job.domain_id === claims.domain_id).map(({ job, tasks_summary }) => ({ job, tasks_summary }));
        return response(200, { items: q.has('cursor') ? [] : items.slice(0, Math.min(2, Number(q.get('limit')))), next_cursor: q.has('cursor') ? null : CURSOR });
      }
      const match = /^\/v1\/jobs\/([0-9a-f-]+)(\/cancel)?$/.exec(path);
      assert.ok(match && uuid.test(match[1]), 'unexpected jobs route'); assert.equal(url.search, '');
      const details = jobs.get(match[1]);
      if (!details || details.job.domain_id !== claims.domain_id) return response(404, {});
      if (method === 'POST' && match[2]) {
        assert.equal(body, undefined); state.cancellations.push(match[1]);
        details.job.status = 'canceled'; details.tasks[0].cancel_requested_at = timestamp;
        return response(200, { id: match[1], status: 'canceled', updated_at: timestamp });
      }
      assert.equal(method, 'GET'); assert.equal(match[2], undefined);
      return response(200, structuredClone(details));
    } catch (error) { state.violations.push(error.message); return response(422, { error: 'Synthetic fixture contract violation' }); }
  };
  return { state, jobs, grant, dispatch };
}
export async function startJobsFixture() {
  const model = createJobsModel(), pending = new Set();
  const fixture = await startFixture({ primaryRoute: async ({ req, res, url, dataBase, records, contents, hold }) => {
    const auth = /^\/api\/v1\/domains\/([^/]+)\/auth$/.exec(url.pathname);
    if (!auth && !url.pathname.startsWith('/v1/jobs')) return false;
    const send = (status, value) => { res.writeHead(status, { 'Content-Type': 'application/json' }); res.end(JSON.stringify(value)); };
    if (auth) {
      if (req.method !== 'POST' || req.headers.authorization !== 'Bearer synthetic-service' || ![DOMAIN, OTHER].includes(auth[1])) send(403, {});
      else send(200, model.grant(auth[1], dataBase));
      return true;
    }
    const chunks = []; let size = 0;
    for await (const chunk of req) { size += chunk.length; if (size > 1024 * 1024) { send(413, {}); return true; } chunks.push(chunk); }
    let body;
    try { if (size) body = JSON.parse(Buffer.concat(chunks)); } catch { send(400, {}); return true; }
    const result = model.dispatch({ method: req.method, url, headers: req.headers, body }, { records, contents });
    if (model.state.hold && (model.state.hold === 'all' || url.pathname.endsWith(model.state.hold))) {
      const exchange = req.fixtureExchange; pending.add(exchange);
      await hold(exchange); pending.delete(exchange);
    }
    if (!res.destroyed) send(result.status, result.body);
    return true;
  } });
  fixture.records.push({ id: INPUT, domain_id: DOMAIN, name: 'Synthetic jobs input', data_type: 'example.text.v1', size: 21, created_at: timestamp, updated_at: timestamp });
  fixture.contents.set(INPUT, Buffer.from('Synthetic jobs input\n'));
  const release = () => { model.state.hold = undefined; for (const exchange of pending) exchange.release(); };
  return { ...fixture, model, release, close: async () => { release(); await fixture.close(); } };
}
