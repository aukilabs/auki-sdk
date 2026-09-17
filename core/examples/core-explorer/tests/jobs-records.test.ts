import { test } from 'node:test';
import assert from 'node:assert/strict';
import { JobsRecords } from '../src/jobs-records.ts';
const domain = 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa', id = 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb';
const metadata = { id, domain_id: domain, name: 'Named input', size: 4 };
function setup(overrides: object = {}) {
  const calls: unknown[] = [];
  const data: any = { list: async (query: unknown) => { calls.push(query); return [metadata]; }, get: async () => metadata,
    readTo: async (_id: string, sink: (bytes: Uint8Array) => void, options: unknown) => { calls.push(options); sink(new TextEncoder().encode('test')); return 4; }, ...overrides };
  let context: any = { domainId: domain, data };
  return { reader: new JobsRecords(() => context), calls, switch: () => { context = undefined; } };
}
test('named picker uses SDK name query and bounded preview without owning client', async () => {
  const { reader, calls } = setup();
  assert.equal((await reader.list('Named input'))?.[0].name, 'Named input');
  assert.deepEqual(calls[0], { name: 'Named input' });
  assert.equal((await reader.preview(id))?.text, 'test');
  assert.deepEqual(calls[1], { maxBytes: 65536, maxChunkBytes: 65536 }); await reader.close();
});
for (const patch of [{ domain_id: id }, { id: domain }, { size: 65537 }, { size: -1 }, { size: 2 }]) test(`picker rejects mismatched or unbounded record ${JSON.stringify(patch)}`, async () => {
  const { reader } = setup({ get: async () => ({ ...metadata, ...patch }) });
  await assert.rejects(reader.preview(id)); await reader.close();
});
test('picker close aborts, drains late reads and fences results across Domain change', async () => {
  let resolve!: (value: any) => void, signal!: AbortSignal;
  const { reader, switch: change } = setup({ list: (_query: unknown, s: AbortSignal) => { signal = s; return new Promise(r => { resolve = r; }); } });
  const pending = reader.list(); change(); let closed = false;
  const closing = reader.close().then(() => { closed = true; });
  assert.equal(signal.aborted, true); await Promise.resolve(); assert.equal(closed, false);
  resolve([metadata]); assert.equal(await pending, undefined); await closing; assert.equal(closed, true);
});
test('picker redacts preview and excludes wrong Domain list entries', async () => {
  const content = new TextEncoder().encode('{"password":"fixture-secret"}');
  const { reader } = setup({ list: async () => [metadata, { ...metadata, domain_id: id }], get: async () => ({ ...metadata, size: content.length }), readTo: async (_id: string, sink: any) => { sink(content); return content.length; } });
  assert.equal((await reader.list())?.length, 1);
  assert.ok(!(await reader.preview(id))?.text.includes('fixture-secret')); await reader.close();
});
