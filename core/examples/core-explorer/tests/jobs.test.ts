import { test } from 'node:test';
import assert from 'node:assert/strict';
import { JobsController, capability, MAX_INPUT_BYTES, type JobsContext, type JobsPort, type DataPort, type DemoConfig } from '../src/jobs.ts';
import type { JobDetails } from '../../portable-echo/web/pkg-web/auki_portable_echo_web.js';

const domain = 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa';
const input = 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb';
const job = 'cccccccc-cccc-4ccc-8ccc-cccccccccccc';
const task = 'dddddddd-dddd-4ddd-8ddd-dddddddddddd';
const config: DemoConfig = { installationId: domain, computeId: input, robotId: task };
const estimate = { total: '123456789012345678.000001', tasks: [] };
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(yes => { resolve = yes; });
  return { promise, resolve };
}
function details(role = 'compute'): JobDetails {
  return {
    job: { id: job, domain_id: domain, label: 'example', status: 'completed', meta: { installation_id: config.installationId }, credit_released_at: null },
    tasks_summary: { completed: 1, queued: 0, running: 0, failed: 0, canceled: 0, leased: 0 },
    tasks: [{ id: task, job_id: job, capability: capability(config.installationId, role as 'compute'), mode: 'dedicated', status: 'completed', reserved_by: null, meta: { input_id: input, run_id: config.installationId }, inputs_cids: [input], attempts: 1, max_attempts: 1 }],
    receipts: [{ id: domain, task_id: task, job_id: job, node_id: role === 'compute' ? config.computeId : config.robotId, outputs: [input], meta: { data_id: input, run_id: config.installationId, sha256: 'a'.repeat(64), bytes: 2 } }],
  } as JobDetails;
}
function harness(overrides: Partial<JobsPort> = {}, dataOverrides: Partial<DataPort> = {}) {
  const calls: string[] = [];
  const submitted: unknown[] = [];
  let signal: AbortSignal | null | undefined;
  const data: DataPort = {
    get: async () => ({ id: input, domain_id: domain, size: 2, name: 'input', data_type: 'text', created_at: '', updated_at: '' }),
    readTo: async (_id, sink, options, abort) => {
      calls.push('read'); assert.equal(options?.maxBytes, MAX_INPUT_BYTES); assert.ok(abort);
      await sink(new Uint8Array([104, 105]), abort!); return 2;
    }, ...dataOverrides,
  };
  const port: JobsPort = {
    estimate: async (spec, abort) => { calls.push('estimate'); assert.ok(abort); assert.equal(spec.tasks[0].mode, 'dedicated'); assert.equal(spec.tasks[0].maxAttempts, 1); return estimate; },
    submit: async (spec, abort) => { calls.push('submit'); signal = abort; submitted.push(spec); return job; },
    get: async () => details(), cancel: async () => { calls.push('cancel'); return { id: job, status: 'canceled', updated_at: '' }; },
    list: async () => ({ items: [], next_cursor: 'opaque-next' }),
    close: async () => { calls.push('close'); }, ...overrides,
  };
  let context: JobsContext | undefined = { domainId: domain, environment: 'fixtures', session: {}, data, createJobs: () => { calls.push('create'); return port; } };
  const controller = new JobsController(() => context);
  return { controller, calls, submitted, port, get signal() { return signal; }, get context() { return context; }, set context(value) { context = value; } };
}

for (const role of ['compute', 'robot'] as const) test(`${role}: exact review, decimal strings, single confirmation and verified outputs`, async () => {
  const h = harness({ get: async () => details(role) });
  await h.controller.configure(config); assert.deepEqual(h.calls, []);
  await h.controller.prepare(role, input);
  assert.equal(h.controller.state.phase, 'review'); assert.equal(h.controller.state.estimate?.total, estimate.total);
  assert.equal(h.controller.state.spec?.tasks[0].capability, capability(config.installationId, role));
  assert.deepEqual(h.controller.state.spec?.tasks[0].meta, { input_id: input, run_id: config.installationId });
  assert.equal(h.submitted.length, 0);
  const first = h.controller.submit(); assert.equal(first, h.controller.submit()); await first; await h.controller.submit();
  assert.equal(h.submitted.length, 1); assert.equal(h.controller.state.executorMatch, true);
  assert.deepEqual(h.controller.state.outputs, [input]); await h.controller.close();
});

test('wrong or missing executor and retry receipts never authorize outputs', async () => {
  for (const node of [null, job]) {
    const d = details(); d.receipts[0].node_id = node;
    const h = harness({ get: async () => d }); await h.controller.configure(config); await h.controller.inspect(job);
    assert.equal(h.controller.state.executorMatch, false); assert.deepEqual(h.controller.state.outputs, []); await h.controller.close();
  }
  const d = details(); d.tasks[0].attempts = 2;
  const h = harness({ get: async () => d }); await h.controller.configure(config); await h.controller.inspect(job);
  assert.deepEqual(h.controller.state.outputs, []); await h.controller.close();
});

test('invalid metadata and non UTF-8 fail before estimate', async () => {
  for (const size of [-1, MAX_INPUT_BYTES + 1]) {
    const h = harness({}, { get: async () => ({ id: input, domain_id: domain, size } as never) });
    await h.controller.configure(config); await h.controller.prepare('compute', input);
    assert.equal(h.controller.state.phase, 'choose'); assert.deepEqual(h.calls, []);
  }
  const h = harness({}, { readTo: async (_id, sink) => { await sink(new Uint8Array([255, 255])); return 2; } });
  await h.controller.configure(config); await h.controller.prepare('compute', input);
  assert.match(h.controller.state.message, /UTF-8/); assert.equal(h.calls.includes('estimate'), false);
});

test('ambiguous submission retains reconciliation and cannot be retried by edits or history', async () => {
  let count = 0;
  const h = harness({ submit: async () => { count++; throw { code: 'submission_uncertain', kind: 'submission_uncertain', message: 'secret=do-not-display' }; } });
  await h.controller.configure(config); await h.controller.prepare('compute', input); const label = h.controller.state.spec!.label;
  await h.controller.submit(); assert.equal(h.controller.state.phase, 'uncertain'); assert.equal(h.controller.state.errorCode, 'submission_uncertain');
  assert.deepEqual(h.controller.state.reconciliation, { label, domainId: domain, environment: 'fixtures' });
  await h.controller.list(); await h.controller.configure(config); await h.controller.prepare('robot', input); await h.controller.submit();
  assert.equal(count, 1); assert.equal(h.controller.state.phase, 'uncertain'); assert.doesNotMatch(h.controller.state.message, /do-not-display/);
  await h.controller.close(); assert.equal(h.controller.state.reconciliation, undefined);
});

test('denial preserves code with a fixed safe message and no review retry', async () => {
  const h = harness({ submit: async () => { throw { code: 'http_status', status: 403, message: 'password=private' }; } });
  await h.controller.configure(config); await h.controller.prepare('compute', input); await h.controller.submit();
  assert.equal(h.controller.state.phase, 'choose'); assert.equal(h.controller.state.errorCode, 'http_status');
  assert.match(h.controller.state.message, /write permission/); assert.doesNotMatch(h.controller.state.message, /private/);
  await h.controller.submit(); await h.controller.close();
});

test('input edit invalidates a delayed estimate and cannot publish stale review', async () => {
  const delayed = deferred<typeof estimate>(), entered = deferred<void>();
  const h = harness({ estimate: async () => { entered.resolve(); return delayed.promise; } });
  await h.controller.configure(config); const p = h.controller.prepare('compute', input); await entered.promise;
  await h.controller.prepare('robot', 'invalid'); delayed.resolve(estimate); await p;
  assert.equal(h.controller.state.estimate, undefined); assert.equal(h.controller.state.spec, undefined);
  await h.controller.close();
});

test('close aborts and drains in-flight submission, clears session state, never cancels the job', async () => {
  const delayed = deferred<string>(), entered = deferred<void>(); let signal: AbortSignal | null | undefined;
  const h = harness({ submit: async (_spec, abort) => { signal = abort; entered.resolve(); return delayed.promise; } });
  await h.controller.configure(config); await h.controller.prepare('compute', input);
  const p = h.controller.submit(); await entered.promise;
  let closed = false; const closing = h.controller.close().then(() => { closed = true; });
  assert.equal(signal?.aborted, true); assert.equal(h.controller.state.config, undefined); await Promise.resolve(); assert.equal(closed, false);
  delayed.resolve(job); await Promise.all([p, closing]);
  assert.equal(h.controller.state.jobId, undefined); assert.equal(h.calls.includes('cancel'), false);
  h.context = { ...h.context!, session: {} }; await h.controller.configure(config); await h.controller.prepare('robot', input);
  assert.equal(h.calls.filter(x => x === 'create').length, 2); await h.controller.close();
});

test('Domain and session changes fence late reads without host close', async () => {
  for (const change of ['domain', 'session']) {
    const delayed = deferred<JobDetails>(), entered = deferred<void>();
    const h = harness({ get: async () => { entered.resolve(); return delayed.promise; } });
    await h.controller.configure(config); const p = h.controller.inspect(job); await entered.promise;
    h.context = { ...h.context!, ...(change === 'domain' ? { domainId: input } : { session: {} }) };
    delayed.resolve(details()); await p; await h.controller.close();
    assert.equal(h.controller.state.details, undefined); assert.equal(h.controller.state.config, undefined);
  }
});

test('history is one bounded filtered page with unchanged cursor and incomplete-list warning', async () => {
  const queries: unknown[] = [];
  const h = harness({ list: async query => { queries.push(query); return { items: [], next_cursor: 'opaque-next' }; } });
  await h.controller.configure(config); await h.controller.list(); await h.controller.list(h.controller.state.nextCursor);
  assert.equal(queries.length, 2); assert.deepEqual(queries[1], { limit: 25, cursor: 'opaque-next', capabilities: [capability(domain, 'compute'), capability(domain, 'robot')], matchAllCapabilities: false });
  assert.match(h.controller.state.message, /#396/); await h.controller.close();
});

test('cancel is deduplicated and refresh preserves running tasks and unreleased credits', async () => {
  const d = details(); d.job.status = 'canceled'; d.tasks[0].status = 'running';
  const h = harness({ get: async () => d }); await h.controller.configure(config); await h.controller.inspect(job);
  const first = h.controller.cancel(); assert.equal(first, h.controller.cancel()); await first;
  assert.equal(h.calls.filter(x => x === 'cancel').length, 1); assert.equal(h.controller.state.details?.tasks[0].status, 'running');
  assert.equal(h.controller.state.details?.job.credit_released_at, null); assert.match(h.controller.state.message, /does not prove/);
  await h.controller.close();
});

test('invalid configuration cannot erase an uncertain submission', async () => {
  const h = harness({ submit: async () => { throw { code: 'submission_uncertain' }; } });
  await h.controller.configure(config); await h.controller.prepare('compute', input); await h.controller.submit();
  const saved = h.controller.state.reconciliation;
  await h.controller.configure({ ...config, installationId: 'invalid' });
  await h.controller.prepare('compute', input);
  assert.deepEqual(h.controller.state.reconciliation, saved);
  await h.controller.configure(config); await h.controller.prepare('compute', input);
  assert.equal(h.controller.state.phase, 'uncertain'); await h.controller.close();
});

test('contradictory reservation, failed receipt and wrong Domain withhold outputs', async () => {
  for (const field of ['reservation', 'receipt', 'domain']) {
    const d = details();
    if (field === 'reservation') d.tasks[0].reserved_by = job;
    if (field === 'receipt') d.receipts[0].meta.status = 'failed';
    if (field === 'domain') d.job.domain_id = input;
    const h = harness({ get: async () => d }); await h.controller.configure(config); await h.controller.inspect(job);
    assert.equal(h.controller.state.outputs?.length ?? 0, 0); await h.controller.close();
  }
});

test('read signals abort on replacement and review intent is private from rendering mutation', async () => {
  let oldSignal: AbortSignal | null | undefined;
  const delayed = deferred<typeof estimate>(), entered = deferred<void>();
  const h = harness({ estimate: async (_spec, signal) => { oldSignal = signal; entered.resolve(); return delayed.promise; } });
  await h.controller.configure(config); const pending = h.controller.prepare('compute', input); await entered.promise;
  await h.controller.configure({ ...config, computeId: job }); assert.equal(oldSignal?.aborted, true);
  delayed.resolve(estimate); await pending; assert.equal(h.controller.state.estimate, undefined); await h.controller.close();
  const fresh = harness(); await fresh.controller.configure(config); await fresh.controller.prepare('compute', input);
  fresh.controller.state.spec!.tasks[0].capability = 'tampered'; await fresh.controller.submit();
  assert.equal((fresh.submitted[0] as { tasks: { capability: string }[] }).tasks[0].capability, capability(domain, 'compute'));
  await fresh.controller.close();
});

test('receipt and task contradictions withhold outputs', async () => {
  const mutations: Array<(d: JobDetails) => void> = [
    d => { d.tasks[0].meta.run_id = job; },
    d => { d.tasks[0].meta.input_id = job; },
    d => { d.tasks[0].inputs_cids = [job]; },
    d => { d.job.meta.installation_id = job; },
    d => { d.receipts[0].meta.run_id = job; },
    d => { d.receipts[0].meta.data_id = job; },
    d => { d.receipts[0].meta.sha256 = 'invalid'; },
    d => { d.receipts[0].meta.bytes = -1; },
    d => { d.receipts[0].meta.bytes = MAX_INPUT_BYTES + 1; },
    d => { d.receipts[0].meta.status = 'canceled'; },
    d => { d.receipts[0].meta.source = 'runtime'; },
    d => { d.receipts[0].outputs = ['opaque']; d.receipts[0].meta.data_id = 'opaque'; },
    d => { d.job.status = 'failed'; },
    d => { d.receipts[0].task_id = job; },
    d => { d.receipts[0].job_id = input; },
  ];
  for (const mutate of mutations) {
    const d = details(); mutate(d);
    const h = harness({ get: async () => d });
    await h.controller.configure(config); await h.controller.inspect(job);
    assert.deepEqual(h.controller.state.outputs, []); await h.controller.close();
  }
});

test('cleanup rejection propagates after requests drain and local state clears', async () => {
  const h = harness({ close: async () => { throw new Error('private cleanup failure'); } });
  await h.controller.configure(config); await h.controller.prepare('compute', input);
  await assert.rejects(h.controller.close(), /^Error: Jobs cleanup failed\.$/);
  assert.equal(h.controller.state.config, undefined); assert.equal(h.controller.state.spec, undefined);
  assert.equal(h.calls.includes('cancel'), false);
});

for (const inFlight of [false, true]) test(`same-session A to B to A preserves ${inFlight ? 'in-flight' : 'uncertain'} intent, logout wipes it`, async () => {
  const entered = deferred<void>(), delayed = deferred<string>();
  const h = harness({ submit: async () => { entered.resolve(); if (inFlight) return delayed.promise; throw { code: 'submission_uncertain' }; } });
  const original = h.context!;
  await h.controller.configure(config); await h.controller.prepare('compute', input);
  const label = h.controller.state.spec!.label;
  const submission = h.controller.submit(); await entered.promise;
  if (!inFlight) await submission;
  h.context = { ...original, domainId: input };
  const closing = h.controller.close(original.session);
  delayed.resolve(job); await Promise.all([submission, closing]);
  assert.equal(h.controller.state.reconciliation, undefined); assert.equal(h.controller.state.config, undefined);
  await h.controller.configure(config); await h.controller.prepare('compute', input);
  assert.equal(h.controller.state.phase, 'choose'); assert.equal(h.controller.state.spec, undefined);
  await h.controller.close(original.session);
  h.context = original; h.controller.restoreContext();
  assert.equal(h.controller.state.phase, 'uncertain'); assert.equal(h.controller.state.reconciliation?.label, label);
  assert.deepEqual(h.controller.state.config, config);
  await h.controller.prepare('robot', input); await h.controller.submit();
  assert.equal(h.controller.state.phase, 'uncertain');
  await h.controller.close(); h.context = { ...original, session: {}, domainId: input };
  h.controller.restoreContext(); assert.equal(h.controller.state.reconciliation, undefined); assert.equal(h.controller.state.config, undefined);
  h.context = { ...original, session: h.context.session };
  await h.controller.configure(config); await h.controller.prepare('compute', input);
  assert.equal(h.controller.state.phase, 'review'); await h.controller.close();
});

test('foreign history identity cannot reconcile a matching label; correct queued intent can', async () => {
  const d = details();
  const h = harness({ submit: async () => { throw { code: 'submission_uncertain' }; }, get: async () => d });
  await h.controller.configure(config); await h.controller.prepare('compute', input);
  d.job.label = h.controller.state.spec!.label;
  await h.controller.submit();
  d.tasks[0].meta.input_id = job; await h.controller.inspect(job);
  assert.ok(h.controller.state.reconciliation);
  d.tasks[0].meta.input_id = input; d.tasks[0].status = 'queued'; d.tasks[0].attempts = 0; d.receipts = [];
  await h.controller.inspect(job); assert.equal(h.controller.state.reconciliation, undefined);
  await h.controller.configure(config); await h.controller.prepare('robot', input);
  assert.equal(h.controller.state.phase, 'review'); await h.controller.close();
});

test('logout during Domain cleanup erases retained in-flight recovery before new account', async () => {
  const entered = deferred<void>(), delayed = deferred<string>();
  const h = harness({ submit: async () => { entered.resolve(); return delayed.promise; } });
  const original = h.context!;
  await h.controller.configure(config); await h.controller.prepare('compute', input);
  const submission = h.controller.submit(); await entered.promise;
  h.context = undefined;
  const switching = h.controller.close(original.session), logout = h.controller.close();
  delayed.resolve(job); await Promise.all([submission, switching, logout]);
  h.context = { ...original, session: {} }; h.controller.restoreContext();
  assert.equal(h.controller.state.reconciliation, undefined); assert.equal(h.controller.state.config, undefined);
  await h.controller.configure(config); await h.controller.prepare('compute', input);
  assert.equal(h.controller.state.phase, 'review'); await h.controller.close();
});

test('missing discovered roles fail before estimation while the other role remains usable', async () => {
  const h = harness();
  await h.controller.configure({ installationId: config.installationId, robotId: config.robotId });
  await h.controller.prepare('compute', input); assert.match(h.controller.state.message, /missing or ambiguous/);
  assert.deepEqual(h.calls, []); await h.controller.submit(); assert.equal(h.submitted.length, 0);
  await h.controller.prepare('robot', input); assert.equal(h.controller.state.phase, 'review'); await h.controller.close();
});
test('rediscovery revokes old review and Back/new-job eligibility while retaining history IDs', async () => {
  const h = harness(); await h.controller.configure(config); await h.controller.prepare('compute', input);
  h.controller.invalidateDiscovery();
  assert.equal(h.controller.state.spec, undefined); assert.equal(h.controller.state.estimate, undefined);
  await h.controller.submit(); assert.equal(h.submitted.length, 0);
  h.controller.choose(); await h.controller.prepare('compute', input);
  assert.equal(h.calls.filter(c => c === 'estimate').length, 1);
  assert.match(h.controller.state.message, /Discover/);
  await h.controller.list(); assert.equal(h.controller.state.phase, 'history');
  assert.deepEqual(h.controller.state.config, config);
  await h.controller.inspect(job); assert.equal(h.controller.state.executorMatch, true);
  h.controller.choose(); await h.controller.prepare('robot', input);
  assert.equal(h.calls.filter(c => c === 'estimate').length, 1);
  await h.controller.configure(config); await h.controller.prepare('robot', input);
  assert.equal(h.controller.state.phase, 'review'); await h.controller.close();
});
test('rediscovery invalidation retains uncertainty and reconciliation history', async () => {
  const h = harness({ submit: async () => { throw { code: 'submission_uncertain' }; } });
  await h.controller.configure(config); await h.controller.prepare('compute', input); await h.controller.submit();
  const reconciliation = h.controller.state.reconciliation;
  h.controller.invalidateDiscovery(); h.controller.choose(); await h.controller.list();
  assert.deepEqual(h.controller.state.reconciliation, reconciliation); assert.deepEqual(h.controller.state.config, config);
  await h.controller.prepare('robot', input); await h.controller.submit();
  assert.equal(h.controller.state.phase, 'uncertain'); assert.equal(h.calls.filter(c => c === 'estimate').length, 1);
  await h.controller.close();
});
test('rediscovery aborts and fences an in-flight estimate', async () => {
  const entered = deferred<void>(), delayed = deferred<typeof estimate>();
  let signal: AbortSignal | null | undefined;
  const h = harness({ estimate: async (_spec, abort) => { signal = abort; entered.resolve(); return delayed.promise; } });
  await h.controller.configure(config);
  const preparing = h.controller.prepare('compute', input); await entered.promise;
  h.controller.invalidateDiscovery(); assert.equal(signal?.aborted, true);
  delayed.resolve(estimate); await preparing;
  assert.equal(h.controller.state.spec, undefined); assert.equal(h.controller.state.estimate, undefined);
  assert.equal(h.controller.state.discoveryRequired, true);
  await h.controller.submit(); assert.equal(h.submitted.length, 0); await h.controller.close();
});
