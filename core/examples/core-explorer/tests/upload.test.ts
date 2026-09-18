import { test } from 'node:test';
import assert from 'node:assert/strict';
import { UploadController, DEFAULT_UPLOAD_TYPE, MAX_UPLOAD_BYTES, validateUpload, type UploadBinding, type UploadClient, type UploadFile } from '../src/upload.ts';
import type { DataMetadata } from '../../portable-echo/web/pkg-web/auki_portable_echo_web.js';

const domainId = 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa';
const unique = 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb';
const target = `core-explorer-${unique}`;
const dataType = 'application.octet-stream';
const bytes = new Uint8Array([0, 255, 128, 42]);
const file = (): UploadFile => ({ name: 'sample.bin', size: bytes.length, arrayBuffer: async () => bytes.slice().buffer });
const metadata = (): DataMetadata => ({ id: 'cccccccc-cccc-4ccc-8ccc-cccccccccccc', domain_id: domainId, name: target, data_type: dataType, size: bytes.length, created_at: '', updated_at: '' });
function deferred<T>() {
  let resolve!: (value: T) => void, reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
function harness(client: Partial<UploadClient> = {}, timeout = 1000) {
  const calls: string[] = [];
  let uploaded = 0;
  let binding: UploadBinding | undefined = { domainId, domainName: 'Original Domain', environment: 'Local fixtures', data: {
    write: async (named, actual) => { calls.push('write'); assert.deepEqual(named, { name: target, dataType }); assert.deepEqual(actual, bytes); return metadata(); },
    get: async id => { calls.push('get'); assert.equal(id, metadata().id); return metadata(); }, ...client,
  } };
  const controller = new UploadController(() => binding, () => {}, () => { uploaded++; }, () => unique, timeout);
  return { controller, calls, get uploaded() { return uploaded; }, get binding() { return binding; }, set binding(value) { binding = value; } };
}

test('review freezes the exact destination and no SDK request occurs until confirmation', async () => {
  const h = harness();
  const review = h.controller.review(file(), dataType);
  assert.equal(Object.isFrozen(review), true); assert.equal(review.environment, 'Local fixtures');
  assert.equal(review.target, target); assert.equal(review.size, 4); assert.deepEqual(h.calls, []);
  const first = h.controller.confirm(), duplicate = h.controller.confirm();
  assert.equal(first, duplicate); await first; await h.controller.confirm();
  assert.deepEqual(h.calls, ['write', 'get']); assert.equal(h.uploaded, 1);
  assert.equal(h.controller.state.outcome, 'verified');
});

test('size and SDK named-type constraints reject invalid files before any read or request', () => {
  const h = harness();
  for (const size of [0, -1, MAX_UPLOAD_BYTES + 1, NaN, 1.5]) {
    assert.throws(() => h.controller.review({ ...file(), size }, dataType));
  }
  for (const value of ['', ' ', 'application/octet-stream', 'a\n', 'x'.repeat(129), 'é'.repeat(65), 'bad;type', 'bad"type']) {
    assert.throws(() => h.controller.review(file(), value));
  }
  validateUpload({ ...file(), size: MAX_UPLOAD_BYTES }, dataType);
  validateUpload(file(), 'my-app.report.v1'); assert.deepEqual(h.calls, []);
});

test('write response alone never reports success; metadata readback must finish first', async () => {
  const readback = deferred<DataMetadata>(), entered = deferred<void>();
  const h = harness({ get: async () => { entered.resolve(); return readback.promise; } });
  h.controller.review(file(), dataType); const pending = h.controller.confirm(); await entered.promise;
  assert.equal(h.controller.state.step, 'sending'); assert.equal(h.controller.state.outcome, undefined); assert.equal(h.uploaded, 0);
  readback.resolve(metadata()); await pending;
  assert.equal(h.controller.state.outcome, 'verified'); assert.equal(h.uploaded, 1);
});

for (const field of ['id', 'domain_id', 'name', 'data_type', 'size'] as const) {
  test(`metadata ${field} mismatch is uncertain and retains target without retry`, async () => {
    const h = harness({ get: async () => ({ ...metadata(), [field]: field === 'size' ? 999 : 'different' }) });
    h.controller.review(file(), dataType); await h.controller.confirm(); await h.controller.confirm();
    assert.equal(h.controller.state.outcome, 'uncertain'); assert.equal(h.uploaded, 0);
    assert.equal(h.controller.state.review?.target, target); assert.deepEqual(h.calls, ['write']);
  });
}

for (const [status, outcome] of [[403, 'denied'], [401, 'denied'], [409, 'collision'], [500, 'uncertain']] as const) {
  test(`write HTTP ${status} is ${outcome}, sanitized and never automatically retried`, async () => {
    let writes = 0;
    const h = harness({ write: async () => { writes++; throw { status, message: 'Bearer PRIVATE_TOKEN' }; } });
    h.controller.review(file(), dataType); await h.controller.confirm(); await h.controller.confirm();
    assert.equal(writes, 1); assert.equal(h.controller.state.outcome, outcome);
    assert.equal(h.uploaded, 0); assert.equal(h.controller.state.message.includes('PRIVATE_TOKEN'), false);
    assert.equal(h.controller.state.review?.target, target); assert.deepEqual(h.calls, []);
  });
}

test('lost write response leaves ambiguous completion with no retry or rollback claim', async () => {
  let writes = 0;
  const h = harness({ write: async () => { writes++; throw new Error('secret backend network error'); } });
  h.controller.review(file(), dataType); await h.controller.confirm(); await h.controller.confirm();
  assert.equal(writes, 1); assert.equal(h.controller.state.outcome, 'uncertain');
  assert.match(h.controller.state.message, /server record may exist/); assert.match(h.controller.state.message, /does not roll back/);
});

test('write succeeds but read permission is denied: retain ID and report uncertain verification', async () => {
  const h = harness({ get: async () => { throw { status: 403, message: 'private' }; } });
  h.controller.review(file(), dataType); await h.controller.confirm();
  assert.equal(h.controller.state.outcome, 'uncertain'); assert.equal(h.controller.state.recordId, metadata().id);
  assert.match(h.controller.state.message, /Metadata readback denied/); assert.equal(h.uploaded, 0);
});

test('cancellation during file buffering awaits settlement and prevents a later write', async () => {
  const buffer = deferred<ArrayBuffer>(), entered = deferred<void>();
  const h = harness();
  h.controller.review({ ...file(), arrayBuffer: () => { entered.resolve(); return buffer.promise; } }, dataType);
  const pending = h.controller.confirm(); await entered.promise;
  let cancelled = false;
  const cancelling = h.controller.cancel().then(() => { cancelled = true; });
  await Promise.resolve(); assert.equal(cancelled, false); assert.equal(h.controller.state.outcome, 'cancelled');
  assert.throws(() => h.controller.review(file(), dataType));
  buffer.resolve(bytes.slice().buffer); await Promise.all([pending, cancelling]);
  assert.deepEqual(h.calls, []); assert.equal(h.uploaded, 0); assert.equal(cancelled, true);
});

test('cancellation after send aborts SDK, awaits its settlement and fences late success/readback', async () => {
  const write = deferred<DataMetadata>(), entered = deferred<void>(); let signal!: AbortSignal;
  const h = harness({ write: async (_target, _bytes, s) => { signal = s!; entered.resolve(); return write.promise; } });
  h.controller.review(file(), dataType); const pending = h.controller.confirm(); await entered.promise;
  let cancelled = false;
  const cancelling = h.controller.cancel().then(() => { cancelled = true; });
  assert.equal(signal.aborted, true); assert.equal(h.controller.state.outcome, 'uncertain');
  await Promise.resolve(); assert.equal(cancelled, false);
  write.resolve(metadata()); await Promise.all([pending, cancelling]);
  assert.deepEqual(h.calls, []); assert.equal(h.uploaded, 0); assert.equal(h.controller.state.review?.target, target);
  assert.equal(h.controller.state.outcome, 'uncertain');
});

test('clear before queued operation begins prevents every file read and SDK call', async () => {
  const h = harness(); let reads = 0;
  h.controller.review({ ...file(), arrayBuffer: async () => { reads++; return bytes.buffer; } }, dataType);
  const pending = h.controller.confirm(); h.controller.clear(); await pending;
  assert.equal(reads, 0); assert.deepEqual(h.calls, []); assert.equal(h.controller.state.outcome, 'cancelled');
});

test('late verification after clear and Domain switch cannot notify or replace cancelled state', async () => {
  const readback = deferred<DataMetadata>(), entered = deferred<void>(); let signal!: AbortSignal;
  const h = harness({ get: async (_id, s) => { signal = s!; entered.resolve(); return readback.promise; } });
  h.controller.review(file(), dataType); const pending = h.controller.confirm(); await entered.promise;
  h.controller.clear(); h.binding = { ...h.binding!, domainId: 'other' };
  assert.equal(signal.aborted, true);
  readback.resolve(metadata()); await pending;
  assert.equal(h.uploaded, 0); assert.equal(h.controller.state.outcome, 'uncertain');
  assert.equal(h.controller.state.review?.domainId, domainId);
});

for (const change of ['domain', 'environment', 'client', 'logout'] as const) {
  test(`${change} after review prevents confirmation from sending`, async () => {
    const h = harness(); h.controller.review(file(), dataType);
    if (change === 'logout') h.binding = undefined;
    else h.binding = { ...h.binding!, ...(change === 'domain' ? { domainId: 'other' } : change === 'environment' ? { environment: 'other' } : { data: { ...h.binding!.data } }) };
    await h.controller.confirm(); assert.deepEqual(h.calls, []); assert.equal(h.uploaded, 0);
    assert.equal(h.controller.state.outcome, 'cancelled');
  });
}

test('unannounced Domain change while buffering prevents write', async () => {
  const buffer = deferred<ArrayBuffer>(), entered = deferred<void>(); const h = harness();
  h.controller.review({ ...file(), arrayBuffer: () => { entered.resolve(); return buffer.promise; } }, dataType);
  const pending = h.controller.confirm(); await entered.promise;
  h.binding = { ...h.binding!, domainId: 'other' }; buffer.resolve(bytes.slice().buffer); await pending;
  assert.deepEqual(h.calls, []); assert.equal(h.controller.state.outcome, 'cancelled');
});

test('timeout after send aborts immediately, preserves ownership and fences late completion', async () => {
  const write = deferred<DataMetadata>(), aborted = deferred<void>();
  const h = harness({ write: async (_target, _bytes, signal) => {
    signal!.addEventListener('abort', () => aborted.resolve(), { once: true }); return write.promise;
  } }, 5);
  h.controller.review(file(), dataType); const pending = h.controller.confirm(); await aborted.promise;
  assert.equal(h.controller.state.outcome, 'uncertain'); assert.equal(h.controller.busy, true);
  assert.equal(h.controller.state.review?.target, target);
  write.resolve(metadata()); await pending;
  assert.deepEqual(h.calls, []); assert.equal(h.uploaded, 0); assert.equal(h.controller.state.outcome, 'uncertain');
});

test('changed byte length is rejected without a write', async () => {
  const h = harness(); h.controller.review({ ...file(), arrayBuffer: async () => new ArrayBuffer(2) }, dataType);
  await h.controller.confirm(); assert.equal(h.controller.state.outcome, 'error'); assert.deepEqual(h.calls, []);
});

test('Domain change during write prevents metadata readback even without host clear', async () => {
  const write = deferred<DataMetadata>(), entered = deferred<void>();
  const h = harness({ write: async () => { entered.resolve(); return write.promise; } });
  h.controller.review(file(), dataType); const pending = h.controller.confirm(); await entered.promise;
  h.binding = { ...h.binding!, domainId: 'other' }; write.resolve(metadata()); await pending;
  assert.deepEqual(h.calls, []); assert.equal(h.uploaded, 0); assert.equal(h.controller.state.outcome, 'uncertain');
  assert.equal(h.controller.state.review?.domainId, domainId);
});

test('logout during write aborts and cannot start metadata readback after response', async () => {
  const write = deferred<DataMetadata>(), entered = deferred<void>(); let signal!: AbortSignal;
  const h = harness({ write: async (_target, _bytes, s) => { signal = s!; entered.resolve(); return write.promise; } });
  h.controller.review(file(), dataType); const pending = h.controller.confirm(); await entered.promise;
  h.binding = undefined; h.controller.clear(); const closing = h.controller.cancel();
  assert.equal(signal.aborted, true); write.resolve(metadata()); await Promise.all([pending, closing]);
  assert.deepEqual(h.calls, []); assert.equal(h.uploaded, 0); assert.equal(h.controller.state.outcome, 'uncertain');
});

test('clear fences A to B to A selection even when context identity returns to original', async () => {
  const write = deferred<DataMetadata>(), entered = deferred<void>();
  const h = harness({ write: async () => { entered.resolve(); return write.promise; } });
  const original = h.binding;
  h.controller.review(file(), dataType); const pending = h.controller.confirm(); await entered.promise;
  h.controller.clear(); h.binding = { ...original!, domainId: 'other' }; h.controller.clear(); h.binding = original;
  write.resolve(metadata()); await pending;
  assert.deepEqual(h.calls, []); assert.equal(h.uploaded, 0); assert.equal(h.controller.state.outcome, 'uncertain');
});

test('refresh callback failure does not replace a verified upload with uncertainty', async () => {
  const h = harness();
  const controller = new UploadController(() => h.binding, () => {}, () => { throw new Error('refresh failed'); }, () => unique);
  controller.review(file(), dataType); await controller.confirm();
  assert.equal(controller.state.outcome, 'verified');
});

test('local buffer failure is sanitized and sends nothing', async () => {
  const h = harness();
  h.controller.review({ ...file(), arrayBuffer: async () => { throw new Error('PRIVATE file detail'); } }, dataType);
  await h.controller.confirm(); assert.deepEqual(h.calls, []); assert.equal(h.controller.state.outcome, 'error');
  assert.equal(h.controller.state.message.includes('PRIVATE'), false);
});

test('separate explicit reviews generate fresh unique names', () => {
  const h = harness(); let generated = 0;
  const controller = new UploadController(() => h.binding, () => {}, () => {}, () => (++generated === 1 ? unique : 'dddddddd-dddd-4ddd-8ddd-dddddddddddd'));
  const first = controller.review(file(), dataType); controller.clear(); const second = controller.review(file(), dataType);
  assert.notEqual(first.target, second.target); assert.deepEqual(h.calls, []);
});


test('default named upload type is valid without editing', () => {
  assert.equal(DEFAULT_UPLOAD_TYPE, 'core-explorer.file.v1');
  assert.doesNotThrow(() => validateUpload(file(), DEFAULT_UPLOAD_TYPE));
});

for (const outcome of ['verified', 'uncertain'] as const) {
  test(`account A ${outcome} upload is erased before account B signs in`, async () => {
    const h = harness(outcome === 'uncertain' ? { write: async () => { throw new Error('transport'); } } : {});
    h.controller.review(file(), dataType); await h.controller.confirm();
    assert.equal(h.controller.state.outcome, outcome);
    h.controller.reset(); h.binding = undefined; await h.controller.cancel();
    h.binding = { domainId: 'account-b-domain', domainName: 'B', environment: 'B environment', data: { write: async () => metadata(), get: async () => metadata() } };
    assert.deepEqual(h.controller.state, { step: 'choose', message: 'Choose one file, then review its destination.' });
  });
}

test('auth reset erases delayed account A write and fences publication in account B', async () => {
  const write = deferred<DataMetadata>(), entered = deferred<void>();
  const h = harness({ write: async () => { entered.resolve(); return write.promise; } });
  h.controller.review(file(), dataType); const pending = h.controller.confirm(); await entered.promise;
  h.controller.reset(); const closing = h.controller.cancel();
  h.binding = { ...h.binding!, domainId: 'account-b-domain', domainName: 'B', data: { ...h.binding!.data } };
  assert.equal(h.controller.state.review, undefined); assert.equal(h.controller.state.metadata, undefined);
  write.resolve(metadata()); await Promise.all([pending, closing]);
  assert.deepEqual(h.controller.state, { step: 'choose', message: 'Choose one file, then review its destination.' });
  assert.equal(h.uploaded, 0); assert.deepEqual(h.calls, []);
});
