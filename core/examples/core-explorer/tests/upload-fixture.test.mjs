import test from 'node:test';
import assert from 'node:assert/strict';
import { parseBufferedUpload, storeBufferedUpload, MAX_UPLOAD, DOMAIN, OTHER } from './fixture.mjs';
const boundary = 'auki-aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa';
const contentType = `multipart/form-data; boundary=${boundary}`;
const encode = (bytes, fields = 'name="unique-target"; data-type="fixture.binary.v1"') => Buffer.concat([
  Buffer.from(`--${boundary}\r\nContent-Type: application/octet-stream\r\nContent-Disposition: form-data; ${fields}\r\n\r\n`), bytes, Buffer.from(`\r\n--${boundary}--\r\n`),
]);
test('SDK buffered part preserves binary bytes including CRLF, NUL and invalid UTF-8', () => {
  const bytes = Buffer.from([0, 255, 128, 13, 10, 34, 92, 0]);
  const upload = parseBufferedUpload(contentType, encode(bytes));
  assert.deepEqual(upload, { name: 'unique-target', data_type: 'fixture.binary.v1', bytes });
});
test('buffered boundary accepts exactly 8 MiB and rejects one extra byte', () => {
  assert.equal(parseBufferedUpload(contentType, encode(Buffer.alloc(MAX_UPLOAD))).bytes.length, MAX_UPLOAD);
  assert.throws(() => parseBufferedUpload(contentType, encode(Buffer.alloc(MAX_UPLOAD + 1))), /limit/);
});
test('fixture rejects invented filename fields, missing type, multiple parts and malformed framing', () => {
  for (const fields of ['name="file"; filename="file.bin"', 'name="target"', 'id="existing-id"', 'name="target"; data-type="application/octet-stream"']) {
    assert.throws(() => parseBufferedUpload(contentType, encode(Buffer.from('x'), fields)));
  }
  assert.throws(() => parseBufferedUpload('application/json', Buffer.from('{}')));
  assert.throws(() => parseBufferedUpload(contentType, encode(Buffer.from('x')).subarray(0, -3)));
  assert.throws(() => parseBufferedUpload(contentType, encode(Buffer.from(boundary))));
});
test('named writes return SDK metadata envelope and never replace a collision', () => {
  const records = [], contents = new Map(), timestamp = '2026-09-01T00:00:00Z';
  const upload = parseBufferedUpload(contentType, encode(Buffer.from([0, 255])));
  const result = storeBufferedUpload(records, contents, DOMAIN, upload, timestamp);
  assert.equal(result.status, 200); assert.equal(result.body.data.length, 1);
  const record = result.body.data[0];
  assert.equal(record.domain_id, DOMAIN); assert.equal(record.size, 2);
  assert.equal(record.name, upload.name); assert.equal(record.data_type, upload.data_type);
  assert.deepEqual(contents.get(record.id), upload.bytes);
  assert.equal(storeBufferedUpload(records, contents, DOMAIN, { ...upload, bytes: Buffer.from('replacement') }, timestamp).status, 409);
  assert.deepEqual(contents.get(record.id), upload.bytes); assert.equal(records.length, 1);
  assert.equal(storeBufferedUpload(records, contents, OTHER, upload, timestamp).status, 200);
  assert.equal(records.length, 2);
});
