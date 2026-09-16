// Synthetic inputs only. Two loopback HTTP origins model DDS and Domain Server.
import http from 'node:http';
import { randomUUID } from 'node:crypto';
import { EventEmitter } from 'node:events';
export const DOMAIN = 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa';
export const OTHER = 'dddddddd-dddd-4ddd-8ddd-dddddddddddd';
export const RECORD = 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb';
export const TEXT = '99999999-9999-4999-8999-999999999999';
export const LARGE = 'cccccccc-cccc-4ccc-8ccc-cccccccccccc';
export const OVERSIZE = 'ffffffff-ffff-4fff-8fff-ffffffffffff';
export const PORTAL = 'eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee';
// Matches core/auki-domain-client/src/data.rs::multipart. Never decode file bytes as text.
export const MAX_UPLOAD = 8 * 1024 * 1024;
export function parseBufferedUpload(contentType, body) {
  const match = /^multipart\/form-data; boundary=(auki-[a-zA-Z0-9-]+)$/.exec(contentType ?? '');
  if (!match) throw new Error('Invalid buffered multipart content type');
  const boundary = match[1];
  const separator = Buffer.from('\r\n\r\n');
  const split = body.indexOf(separator);
  const header = split < 0 ? '' : body.subarray(0, split).toString('utf8');
  const prefix = `--${boundary}\r\nContent-Type: application/octet-stream\r\nContent-Disposition: form-data; `;
  if (!header.startsWith(prefix)) throw new Error('Invalid buffered multipart headers');
  const fields = /^name="([^"\r\n]+)"; data-type="([^"\r\n]+)"$/.exec(header.slice(prefix.length));
  const trailer = Buffer.from(`\r\n--${boundary}--\r\n`);
  if (!fields || !body.subarray(-trailer.length).equals(trailer)) throw new Error('Invalid named buffered part');
  const bytes = Buffer.from(body.subarray(split + separator.length, -trailer.length));
  if (bytes.length > MAX_UPLOAD) throw new Error('Buffered upload exceeds limit');
  if (bytes.includes(Buffer.from(boundary))) throw new Error('Multiple parts or colliding boundary');
  if ([fields[1], fields[2]].some(value => Buffer.byteLength(value) > 128)) throw new Error('Name or type exceeds SDK limit');
  if ([fields[1], fields[2]].some(value => /[;!?<>[\]{}()/\\"$#@*^&|~%=+\x00-\x1f\x7f]/.test(value))) throw new Error('Invalid name or type');
  return { name: fields[1], data_type: fields[2], bytes };
}

// Pure fixture storage contract, also exercised without starting a server.
export function storeBufferedUpload(records, contents, domainId, upload, timestamp) {
  if (records.some(record => record.domain_id === domainId && record.name === upload.name)) return { status: 409, body: {} };
  const record = { id: randomUUID(), domain_id: domainId, name: upload.name, data_type: upload.data_type, size: upload.bytes.length, created_at: timestamp, updated_at: timestamp };
  records.push(record); contents.set(record.id, Buffer.from(upload.bytes));
  return { status: 200, body: { data: [record] } };
}

export async function startFixture() {
  const state = { requests: [], denyLogin: false, denyDomains: false, denyPortals: false, denyData: false, emptyDomains: false, holdReads: false, holdLogin: false, aborted: 0, exchanges: [], uploads: [], denyWrite: false, collideWrite: false, failWrite: false, dropWriteResponse: false, holdWrites: false, holdVerification: false, denyVerification: false };
  const events = new EventEmitter();
  const changed = () => events.emit('change');
  const waitFor = predicate => new Promise((resolve, reject) => {
    const check = () => { if (predicate()) { cleanup(); resolve(); } };
    const timer = setTimeout(() => { cleanup(); reject(new Error('Fixture synchronization timed out')); }, 5000);
    const cleanup = () => { clearTimeout(timer); events.off('change', check); };
    events.on('change', check); check();
  });
  const hold = async exchange => {
    exchange.held = true;
    await new Promise(resolve => { exchange.release = resolve; changed(); });
  };
  const releaseReads = () => { state.holdReads = false; for (const e of state.exchanges) if (e.held && e.method === 'GET') e.release(); };
  const timestamp = '2026-09-01T00:00:00Z';
  const textContent = 'Authorization: Bearer SYNTHETIC_OPAQUE_SECRET\naUtHoRiZaTiOn \t=\t bEaReR\tSYNTHETIC_EQUALS_SECRET\nUseful fixture text';
  const content = JSON.stringify({ message: '<img src=x onerror=alert(1)>', nested: { access_token: 'synthetic-secret', notes: [{ note: textContent }] }, description: 'Synthetic lab sample' });
  const record = { id: RECORD, domain_id: DOMAIN, name: 'Lab sample', data_type: 'fixture.v1', size: Buffer.byteLength(content), created_at: timestamp, updated_at: timestamp };
  const largeContent = JSON.stringify({ nested: { private_key: 'SYNTHETIC_PRIVATE', api_key: 'SYNTHETIC_API', access_token: 'SYNTHETIC_START"escaped\\SYNTHETIC_END' }, tail: 'x'.repeat(70000) });
  const contents = new Map([[RECORD, content], [TEXT, textContent], [LARGE, largeContent], [OVERSIZE, 'x'.repeat(8 * 1024 * 1024 + 1)]]);
  const records = [record, { ...record, id: TEXT, name: 'Bearer text', size: Buffer.byteLength(textContent) }, { ...record, id: LARGE, name: 'Large JSON', data_type: 'fixture.large', size: Buffer.byteLength(largeContent) }, { ...record, id: OVERSIZE, name: 'Oversized text', data_type: 'fixture.oversize', size: contents.get(OVERSIZE).length }];
  const portal = { id: PORTAL, short_id: 'ABC12345678', name: 'Lab portal', size: 10, organization_id: null, default_domain_id: DOMAIN, redirect_url: null, created_at: timestamp, updated_at: timestamp };
  const pose = { id: PORTAL, short_id: 'ABC12345678', domain_id: DOMAIN, reported_size: 10, px: 1, py: 2, pz: 3, rx: 0, ry: 0, rz: 0, rw: 1, latitude: null, longitude: null, altitude: null, vertical_accuracy: null, horizontal_accuracy: null, gps_timestamp: null, scanner_device_id: 'fixture-device', scanner_device_name: 'Fixture device', scanner_device_model: 'fixture', placed_at: timestamp };
  const domains = Array.from({ length: 11 }, (_, i) => ({ id: i === 0 ? DOMAIN : i === 10 ? OTHER : `00000000-0000-4000-8000-${String(i).padStart(12, '0')}`, name: i === 0 ? 'Synthetic lab' : i === 10 ? 'Empty lab' : `Fixture ${i}`, organization_id: null }));
  const send = (res, status, value) => { res.writeHead(status, { 'Content-Type': 'application/json' }); res.end(JSON.stringify(value)); };
  const wrap = handler => async (req, res) => {
    res.setHeader('Access-Control-Allow-Origin', '*');
    res.setHeader('Access-Control-Allow-Headers', 'Authorization, Content-Type, Accept, Cache-Control, posemesh-client-id, posemesh-sdk-version');
    res.setHeader('Access-Control-Allow-Methods', 'GET, POST, OPTIONS');
    if (req.method === 'OPTIONS') return send(res, 200, {});
    const url = new URL(req.url, 'http://127.0.0.1');
    state.requests.push(`${req.method} ${url.pathname}${url.search}`);
    const exchange = req.fixtureExchange = { method: req.method, path: url.pathname, held: false, aborted: false, finished: false };
    state.exchanges.push(exchange);
    res.on('finish', () => { exchange.finished = true; changed(); });
    res.on('close', () => { if (!res.writableEnded) { state.aborted++; exchange.aborted = true; exchange.release?.(); } changed(); });
    changed();
    try { await handler(req, res, url); } catch { if (!res.destroyed) send(res, 500, {}); }
  };
  const dataServer = http.createServer(wrap(async (req, res, url) => {
    if (req.method === 'GET' && url.pathname === '/api/v1/info') return send(res, 200, { upload: { domain_data_max_bytes: MAX_UPLOAD, request_max_bytes: MAX_UPLOAD + 4096, multipart: { enabled: false, part_size_bytes: 0 } } });
    if (!req.headers.authorization) return send(res, 401, {});
    const domainId = url.pathname.split('/')[4];
    if (![DOMAIN, OTHER].includes(domainId)) return send(res, 404, {});
    if (req.method === 'POST' && url.pathname === `/api/v1/domains/${domainId}/data`) {
      const chunks = []; let size = 0;
      for await (const chunk of req) {
        size += chunk.length;
        if (size > MAX_UPLOAD + 4096) return send(res, 413, {});
        chunks.push(chunk);
      }
      let upload;
      try { upload = parseBufferedUpload(req.headers['content-type'], Buffer.concat(chunks)); }
      catch { return send(res, 400, {}); }
      const observation = { ...upload, domain_id: domainId, exchange: req.fixtureExchange, stored: false };
      state.uploads.push(observation); changed();
      if (state.denyWrite) return send(res, 403, { secret: 'synthetic-backend-secret' });
      if (state.collideWrite) return send(res, 409, {});
      if (state.failWrite) { res.destroy(); return; }
      const result = storeBufferedUpload(records, contents, domainId, upload, timestamp);
      observation.stored = result.status === 200;
      observation.record = result.body.data?.[0]; changed();
      // Hold after storage: aborting a buffered POST cannot promise rollback.
      if (state.holdWrites) await hold(req.fixtureExchange);
      if (res.destroyed) return;
      if (state.dropWriteResponse) { res.destroy(); return; }
      return send(res, result.status, result.body);
    }
    if (req.method !== 'GET') return send(res, 405, {});
    const uploaded = state.uploads.find(upload => upload.record?.id === url.pathname.split('/').at(-1));
    if (uploaded && url.searchParams.get('raw') !== 'true') {
      if (state.holdVerification) await hold(req.fixtureExchange);
      if (state.denyVerification) return send(res, 403, {});
    }
    if (state.holdReads) await hold(req.fixtureExchange);
    if (res.destroyed) return;
    if (domainId === OTHER && url.pathname.endsWith('/lighthouses')) return send(res, 200, { poses: [] });
    if (url.pathname.endsWith('/lighthouses')) return send(res, 200, { poses: [pose] });
    if (state.denyData) return send(res, 403, { secret: 'synthetic-backend-secret' });
    if (url.pathname.endsWith('/data')) {
      const matches = records.filter(record => record.domain_id === domainId && (!url.searchParams.has('name') || url.searchParams.get('name') === record.name)
        && (!url.searchParams.has('data_type') || url.searchParams.get('data_type') === record.data_type)
        && (!url.searchParams.has('ids') || url.searchParams.get('ids').split(',').includes(record.id)));
      return send(res, 200, { data: matches });
    }
    const selected = records.find(record => record.domain_id === domainId && url.pathname.endsWith(record.id));
    if (selected) {
      if (url.searchParams.get('raw') === 'true') { res.writeHead(200, { 'Content-Type': 'application/octet-stream' }); return res.end(contents.get(selected.id)); }
      return send(res, 200, selected);
    }
    send(res, 404, {});
  }));
  await new Promise(resolve => dataServer.listen(0, '127.0.0.1', resolve));
  const dataBase = `http://127.0.0.1:${dataServer.address().port}`;
  let base;
  const primary = http.createServer(wrap(async (req, res, url) => {
    if (url.pathname === '/user/login') {
      for await (const _ of req) { /* Deliberately do not retain credentials. */ }
      if (state.holdLogin) await hold(req.fixtureExchange);
      if (res.destroyed) return;
      return send(res, state.denyLogin ? 401 : 200, state.denyLogin ? { secret: 'synthetic-backend-secret' } : { access_token: 'synthetic-user', refresh_token: 'synthetic-refresh' });
    }
    if (url.pathname === '/service/domains-access-token') return send(res, 200, { access_token: 'synthetic-service' });
    if (url.pathname === '/api/v1/domains') {
      if (state.denyDomains) return send(res, 403, {});
      const limit = Number(url.searchParams.get('limit')), offset = Number(url.searchParams.get('offset'));
      return send(res, 200, { domains: state.emptyDomains ? [] : domains.slice(offset, offset + limit), total: state.emptyDomains ? 0 : domains.length, limit, offset });
    }
    const id = url.pathname.split('/')[4];
    if (url.pathname.endsWith('/auth')) {
      const claims = { iss: 'dds', domain_id: id, aud: ['dds', dataBase], exp: Math.floor(Date.now() / 1000) + 3600 };
      return send(res, 200, { id, domain_server: { url: dataBase }, access_token: `e30.${Buffer.from(JSON.stringify(claims)).toString('base64url')}.fixture` });
    }
    if (url.pathname.endsWith('/lighthouses')) return send(res, state.denyPortals ? 403 : 200, state.denyPortals ? { secret: 'synthetic-backend-secret' } : { lighthouses: id === OTHER ? [] : [portal] });
    send(res, 404, {});
  }));
  await new Promise(resolve => primary.listen(18114, '127.0.0.1', resolve));
  base = 'http://127.0.0.1:18114';
  return { state, base, dataBase, content, records, contents, waitFor, releaseReads, releaseWrites: () => { state.holdWrites = false; for (const e of state.exchanges) if (e.held && e.method === 'POST') e.release(); }, releaseVerification: () => { state.holdVerification = false; releaseReads(); }, close: async () => { for (const e of state.exchanges) e.release?.(); primary.closeAllConnections(); dataServer.closeAllConnections(); await Promise.all([new Promise(resolve => primary.close(resolve)), new Promise(resolve => dataServer.close(resolve))]); } };
}
if (process.argv[1] === new URL(import.meta.url).pathname) {
  const fixture = await startFixture();
  console.log('Local fixtures / synthetic test data: http://127.0.0.1:18114');
  for (const signal of ['SIGTERM', 'SIGINT']) process.on(signal, () => { void fixture.close().then(() => process.exit()); });
}
