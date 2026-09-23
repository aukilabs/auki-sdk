// Loopback-only API, DDS, and Domain Server fixture for platform binding tests.
// It never forwards requests and records only method/path counters.
import http from 'node:http';
import { randomUUID } from 'node:crypto';

const port = Number(process.argv[2] ?? 18114);
if (!Number.isInteger(port) || port < 1024 || port > 65535) throw new Error('invalid port');
const dataPort = Number(process.env.AUKI_DOMAIN_DATA_SERVER_PORT ?? 0);
if (!Number.isInteger(dataPort) || dataPort < 0 || dataPort > 65535 || dataPort === port) {
  throw new Error('invalid data port');
}

export const DOMAIN_ID = 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa';
export const OTHER_DOMAIN_ID = 'dddddddd-dddd-4ddd-8ddd-dddddddddddd';
export const INITIAL_DATA_ID = 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb';
export const PORTAL_ID = 'eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee';
export const PORTAL_SHORT_ID = 'ABC12345678';
export const UPLOAD_ID = 'cccccccc-cccc-4ccc-8ccc-cccccccccccc';
export const DENIED_DATA_ID = 'ffffffff-ffff-4fff-8fff-ffffffffffff';
const ORGANIZATION_ID = '11111111-1111-4111-8111-111111111111';

const primaryBase = `http://127.0.0.1:${port}`;
const timestamp = '2026-09-01T00:00:00Z';
const initialBytes = Buffer.from('fixture');
let dataBase;
let state;

const metadata = (id, name, dataType, bytes) => ({
  id,
  domain_id: DOMAIN_ID,
  name,
  data_type: dataType,
  size: bytes.length,
  created_at: timestamp,
  updated_at: timestamp,
});

function reset(config = {}) {
  state = {
    config: {
      denyWrites: false,
      serviceUnauthorizedOnce: false,
      ddsUnauthorizedOnce: false,
      domainsUnauthorizedOnce: false,
      accessibleDomainsUnauthorizedOnce: false,
      importedViewer: false,
      denyP2pExchange: false,
      wrongP2pToken: false,
      ...config,
    },
    phase: { phase: 'idle' },
    requests: {},
    logins: 0,
    refreshes: 0,
    importedRefreshes: 0,
    exchanges: 0,
    p2pExchanges: 0,
    domainAuths: 0,
    multipartAborts: 0,
    multipartCompletions: 0,
    generation: 0,
    importedGeneration: 0,
    nextUpload: 0,
    activeP2pToken: null,
    activeImportedServiceToken: null,
    records: new Map([[INITIAL_DATA_ID, {
      metadata: metadata(INITIAL_DATA_ID, 'fixture', 'fixture.v1', initialBytes),
      bytes: initialBytes,
    }]]),
    uploads: new Map(),
  };
}
reset();

function cors(response) {
  response.setHeader('Access-Control-Allow-Origin', '*');
  response.setHeader(
    'Access-Control-Allow-Headers',
    'Authorization, Content-Type, Accept, Cache-Control, posemesh-client-id, posemesh-sdk-version',
  );
  response.setHeader('Access-Control-Allow-Methods', 'GET, POST, PUT, DELETE, OPTIONS');
  response.setHeader('Cache-Control', 'no-store');
}

function json(response, status, value = {}) {
  const bytes = Buffer.from(JSON.stringify(value));
  response.statusCode = status;
  response.setHeader('Content-Type', 'application/json');
  response.setHeader('Content-Length', String(bytes.length));
  response.end(bytes);
}

function bytes(response, status, value = Buffer.alloc(0)) {
  response.statusCode = status;
  response.setHeader('Content-Type', 'application/octet-stream');
  response.setHeader('Content-Length', String(value.length));
  response.end(value);
}

async function readBody(request, maximum = 70 * 1024 * 1024) {
  const chunks = [];
  let length = 0;
  for await (const chunk of request) {
    length += chunk.length;
    if (length > maximum) throw new Error('request too large');
    chunks.push(chunk);
  }
  return Buffer.concat(chunks);
}

function count(request, pathname) {
  const key = `${request.method} ${pathname}`;
  state.requests[key] = (state.requests[key] ?? 0) + 1;
}

function stats() {
  return {
    requests: { ...state.requests },
    logins: state.logins,
    refreshes: state.refreshes,
    importedRefreshes: state.importedRefreshes,
    exchanges: state.exchanges,
    p2pExchanges: state.p2pExchanges,
    domainAuths: state.domainAuths,
    multipartAborts: state.multipartAborts,
    multipartCompletions: state.multipartCompletions,
    outstandingUploads: state.uploads.size,
    records: state.records.size,
    recordIds: [...state.records.keys()],
  };
}

function requireBearer(request, prefix) {
  return String(request.headers.authorization ?? '').startsWith(`Bearer ${prefix}`);
}

function bearerIs(request, token) {
  return request.headers.authorization === `Bearer ${token}`;
}

function dataGrant() {
  const claims = {
    iss: 'dds',
    domain_id: DOMAIN_ID,
    aud: ['dds', dataBase],
    exp: Math.floor(Date.now() / 1000) + 3600,
  };
  return `e30.${Buffer.from(JSON.stringify(claims)).toString('base64url')}.fixture`;
}

function importedServiceToken(type, domains) {
  const now = Math.floor(Date.now() / 1000);
  const claims = {
    type,
    iss: 'api',
    aud: ['domain-service'],
    sub: 'fixture-user',
    org: ORGANIZATION_ID,
    iat: now,
    exp: now + 3600,
  };
  if (domains !== undefined) claims.domains = domains;
  return `e30.${Buffer.from(JSON.stringify(claims)).toString('base64url')}.fixture`;
}

function portal() {
  return {
    id: PORTAL_ID,
    short_id: PORTAL_SHORT_ID,
    name: 'Fixture portal',
    size: 10,
    organization_id: null,
    default_domain_id: DOMAIN_ID,
    redirect_url: null,
    created_at: timestamp,
    updated_at: timestamp,
  };
}

function pose() {
  return {
    id: PORTAL_ID,
    short_id: PORTAL_SHORT_ID,
    domain_id: DOMAIN_ID,
    reported_size: 10,
    px: 1,
    py: 2,
    pz: 3,
    rx: 0,
    ry: 0,
    rz: 0,
    rw: 1,
    latitude: null,
    longitude: null,
    altitude: null,
    vertical_accuracy: null,
    horizontal_accuracy: null,
    gps_timestamp: null,
    scanner_device_id: 'fixture-device',
    scanner_device_name: 'Fixture device',
    scanner_device_model: 'fixture',
    placed_at: timestamp,
  };
}

function parseSimpleMultipart(body, contentType) {
  const boundary = /boundary=([^;]+)/i.exec(contentType)?.[1];
  if (!boundary) return null;
  const headerEnd = body.indexOf('\r\n\r\n');
  const trailer = Buffer.from(`\r\n--${boundary}--\r\n`);
  const trailerAt = body.lastIndexOf(trailer);
  if (headerEnd < 0 || trailerAt < headerEnd + 4) return null;
  const headers = body.subarray(0, headerEnd).toString('utf8');
  const id = /(?:^|;\s*)id="([^"]+)"/.exec(headers)?.[1];
  const name = /(?:^|;\s*)name="([^"]+)"/.exec(headers)?.[1];
  const dataType = /(?:^|;\s*)data-type="([^"]+)"/.exec(headers)?.[1];
  return { id, name, dataType, bytes: body.subarray(headerEnd + 4, trailerAt) };
}

function newDataId() {
  let id = randomUUID();
  while (state.records.has(id)) id = randomUUID();
  return id;
}

async function primaryHandler(request, response) {
  cors(response);
  if (request.method === 'OPTIONS') return json(response, 200);
  const url = new URL(request.url, primaryBase);
  try {
    if (url.pathname === '/__stats' && request.method === 'GET') return json(response, 200, stats());
    if (url.pathname === '/__reset' && request.method === 'POST') {
      const body = await readBody(request);
      reset(body.length ? JSON.parse(body) : {});
      return json(response, 200, { ...stats(), domainId: DOMAIN_ID, dataId: INITIAL_DATA_ID });
    }
    if (url.pathname === '/__phase') {
      if (request.method === 'POST') {
        const body = await readBody(request);
        state.phase = body.length ? JSON.parse(body) : { phase: 'idle' };
      }
      return json(response, 200, state.phase);
    }
    if (url.pathname === '/__configure' && request.method === 'POST') {
      const body = await readBody(request);
      Object.assign(state.config, body.length ? JSON.parse(body) : {});
      return json(response, 200, state.config);
    }
    count(request, url.pathname);
    if (url.pathname === '/user/login' && request.method === 'POST') {
      await readBody(request);
      state.logins++;
      return json(response, 200, { access_token: `user-${state.generation}`, refresh_token: `refresh-${state.generation}` });
    }
    if (url.pathname === '/user/refresh' && request.method === 'POST') {
      await readBody(request);
      if (!bearerIs(request, `refresh-${state.generation}`)) return json(response, 401);
      state.refreshes++;
      state.generation++;
      return json(response, 200, { access_token: `user-${state.generation}`, refresh_token: `refresh-${state.generation}` });
    }
    if (url.pathname === '/.well-known/openid-configuration' && request.method === 'GET') {
      return json(response, 200, {
        issuer: primaryBase,
        token_endpoint: `${primaryBase}/oauth/v2/token`,
        token_endpoint_auth_methods_supported: ['none'],
        grant_types_supported: ['refresh_token'],
      });
    }
    if (url.pathname === '/oauth/v2/token' && request.method === 'POST') {
      const form = new URLSearchParams((await readBody(request)).toString('utf8'));
      const keys = [...form.keys()].sort();
      if (keys.join(',') !== 'client_id,grant_type,refresh_token'
        || form.get('grant_type') !== 'refresh_token'
        || form.get('client_id') !== 'domain-data-public-client'
        || form.get('refresh_token') !== `imported-refresh-${state.importedGeneration}`) {
        return json(response, 400, { error: 'invalid_grant' });
      }
      state.importedRefreshes++;
      state.importedGeneration++;
      return json(response, 200, {
        access_token: `imported-access-${state.importedGeneration}`,
        refresh_token: `imported-refresh-${state.importedGeneration}`,
        expires_in: 3600,
        token_type: 'Bearer',
      });
    }
    if (url.pathname === '/service/domains-access-token' && request.method === 'POST') {
      await readBody(request);
      state.exchanges++;
      if (state.config.serviceUnauthorizedOnce) {
        state.config.serviceUnauthorizedOnce = false;
        return json(response, 401);
      }
      const user = bearerIs(request, `user-${state.generation}`);
      const imported = bearerIs(request, `imported-access-${state.importedGeneration}`);
      if (!user && !imported) return json(response, 401);
      const purpose = url.searchParams.get('purpose');
      if (purpose && purpose !== 'p2p') return json(response, 400);
      if (purpose === 'p2p') {
        state.p2pExchanges++;
        if (!imported) return json(response, 403);
        if (state.config.denyP2pExchange) return json(response, 403);
        state.activeP2pToken = importedServiceToken(
          state.config.wrongP2pToken ? 'user-access' : 'user-p2p-access',
          [DOMAIN_ID, OTHER_DOMAIN_ID],
        );
        return json(response, 200, { access_token: state.activeP2pToken });
      }
      if (imported) {
        state.activeImportedServiceToken = state.config.importedViewer
          ? importedServiceToken('app-access')
          : importedServiceToken('user-access', null);
        return json(response, 200, { access_token: state.activeImportedServiceToken });
      }
      return json(response, 200, {
        access_token: `service-${state.generation}`,
      });
    }
    if (url.pathname === '/api/v1/domains' && request.method === 'GET') {
      const imported = state.activeImportedServiceToken
        && bearerIs(request, state.activeImportedServiceToken);
      if (!imported && !requireBearer(request, 'service-')) return json(response, 401);
      if (state.config.ddsUnauthorizedOnce || state.config.domainsUnauthorizedOnce) {
        state.config.ddsUnauthorizedOnce = false;
        state.config.domainsUnauthorizedOnce = false;
        return json(response, 401);
      }
      const limit = Number(url.searchParams.get('limit') ?? 50);
      const offset = Number(url.searchParams.get('offset') ?? 0);
      if (imported && (state.config.importedViewer
        || url.searchParams.get('org') !== 'own'
        || url.searchParams.get('issue_token') !== 'false'
        || url.searchParams.has('domain_server_id'))) return json(response, 403);
      const all = imported
        ? [
          { id: DOMAIN_ID, name: 'Fixture Domain', description: 'first', organization_id: ORGANIZATION_ID },
          { id: OTHER_DOMAIN_ID, name: 'Other Domain', description: 'second', organization_id: ORGANIZATION_ID },
        ]
        : [{ id: DOMAIN_ID, name: 'Fixture Domain', organization_id: null }];
      return json(response, 200, { domains: all.slice(offset, offset + limit), total: all.length, limit, offset });
    }
    if (url.pathname === '/api/v1/accessible-domains' && request.method === 'GET') {
      const peer = state.activeP2pToken && bearerIs(request, state.activeP2pToken);
      const importedUser = !state.config.importedViewer && state.activeImportedServiceToken
        && bearerIs(request, state.activeImportedServiceToken);
      if (!peer && !importedUser) return json(response, 401);
      if (state.config.accessibleDomainsUnauthorizedOnce) {
        state.config.accessibleDomainsUnauthorizedOnce = false;
        return json(response, 401);
      }
      const limit = Number(url.searchParams.get('limit') ?? 100);
      const offset = Number(url.searchParams.get('offset') ?? 0);
      const all = [
        { id: DOMAIN_ID, name: 'Fixture Domain', description: 'first', organization_id: ORGANIZATION_ID },
        { id: OTHER_DOMAIN_ID, name: 'Other Domain', description: 'second', organization_id: ORGANIZATION_ID },
      ];
      return json(response, 200, {
        domains: all.slice(offset, offset + limit), total: all.length, limit, offset,
      });
    }
    const auth = /^\/api\/v1\/domains\/([^/]+)\/auth$/.exec(url.pathname);
    if (auth && request.method === 'POST') {
      await readBody(request);
      state.domainAuths++;
      if (!requireBearer(request, 'service-')
        && (!state.activeImportedServiceToken
          || !bearerIs(request, state.activeImportedServiceToken))) return json(response, 401);
      if (auth[1] !== DOMAIN_ID) return json(response, 403);
      return json(response, 200, {
        id: DOMAIN_ID,
        domain_server: { url: dataBase },
        access_token: dataGrant(),
      });
    }
    const lighthouse = /^\/api\/v1\/domains\/([^/]+)\/lighthouses(?:\/([^/]+))?$/.exec(url.pathname);
    if (lighthouse && request.method === 'GET') {
      if (!requireBearer(request, 'e30.')) return json(response, 401);
      if (lighthouse[1] !== DOMAIN_ID) return json(response, 403);
      return json(response, 200, lighthouse[2]
        ? { domain_id: DOMAIN_ID, ...portal() }
        : { lighthouses: [portal()] });
    }
    const portalDomains = /^\/api\/v1\/lighthouses\/([^/]+)\/domains$/.exec(url.pathname);
    if (portalDomains && request.method === 'GET') {
      if (!requireBearer(request, 'service-')) return json(response, 401);
      const portalKey = portalDomains[1];
      if (portalKey !== PORTAL_ID && portalKey.toUpperCase() !== PORTAL_SHORT_ID) return json(response, 404);
      return json(response, 200, { domains: [{
        id: DOMAIN_ID,
        name: 'Fixture Domain',
        organization_id: null,
        is_default: true,
        added_to_domain_at: timestamp,
      }] });
    }
    return json(response, 404);
  } catch {
    return json(response, 500, { error: 'synthetic fixture failure' });
  }
}

async function dataHandler(request, response) {
  cors(response);
  if (request.method === 'OPTIONS') return json(response, 200);
  const url = new URL(request.url, dataBase);
  try {
    count(request, url.pathname);
    if (url.pathname === '/api/v1/info' && request.method === 'GET') {
      return json(response, 200, { upload: {
        domain_data_max_bytes: 64 * 1024 * 1024,
        request_max_bytes: 4096,
        multipart: { enabled: true, part_size_bytes: 4 },
      } });
    }
    if (!requireBearer(request, 'e30.')) return json(response, 401);
    const lighthouse = /^\/api\/v1\/domains\/([^/]+)\/lighthouses(?:\/([^/]+))?$/.exec(url.pathname);
    if (lighthouse && request.method === 'GET') {
      if (lighthouse[1] !== DOMAIN_ID) return json(response, 403);
      return json(response, 200, lighthouse[2] ? pose() : { poses: [pose()] });
    }
    const multipart = /^\/api\/v1\/domains\/([^/]+)\/data\/multipart$/.exec(url.pathname);
    if (multipart) {
      if (multipart[1] !== DOMAIN_ID) return json(response, 403);
      if (state.config.denyWrites && request.method !== 'DELETE') return json(response, 403);
      const uploadId = url.searchParams.get('uploadId');
      if (request.method === 'POST' && url.searchParams.has('uploads')) {
        const payload = JSON.parse((await readBody(request)).toString('utf8'));
        const dataId = payload.existing_id ?? newDataId();
        const id = state.nextUpload++ === 0 ? UPLOAD_ID : randomUUID();
        state.uploads.set(id, {
          dataId,
          name: payload.name,
          dataType: payload.data_type,
          size: Number(payload.size),
          parts: new Map(),
        });
        return json(response, 200, {
          upload_id: id,
          data_id: dataId,
          part_size: 4,
          expires_at: new Date(Date.now() + 3600_000).toISOString(),
        });
      }
      const upload = state.uploads.get(uploadId);
      if (!upload) return json(response, 404);
      if (request.method === 'PUT') {
        const part = Number(url.searchParams.get('partNumber'));
        upload.parts.set(part, await readBody(request));
        return json(response, 200, { etag: `part-${part}` });
      }
      if (request.method === 'DELETE') {
        await readBody(request);
        state.uploads.delete(uploadId);
        state.multipartAborts++;
        return bytes(response, 200);
      }
      if (request.method === 'POST') {
        const completion = JSON.parse((await readBody(request)).toString('utf8'));
        const content = Buffer.concat(completion.parts.map(({ part_number: number }) => upload.parts.get(number) ?? Buffer.alloc(0)));
        if (content.length !== upload.size) return json(response, 400);
        const old = state.records.get(upload.dataId);
        const item = metadata(
          upload.dataId,
          upload.name ?? old?.metadata.name ?? 'fixture',
          upload.dataType ?? old?.metadata.data_type ?? 'fixture.v1',
          content,
        );
        state.records.set(upload.dataId, { metadata: item, bytes: content });
        state.uploads.delete(uploadId);
        state.multipartCompletions++;
        return json(response, 200, item);
      }
    }
    const data = /^\/api\/v1\/domains\/([^/]+)\/data(?:\/([^/]+))?$/.exec(url.pathname);
    if (!data) return json(response, 404);
    if (data[1] !== DOMAIN_ID) return json(response, 403);
    const id = data[2];
    if (id === DENIED_DATA_ID) return json(response, 403);
    if (request.method === 'GET' && !id) {
      let records = [...state.records.values()];
      const ids = url.searchParams.get('ids')?.split(',');
      if (ids) records = records.filter((record) => ids.includes(record.metadata.id));
      if (url.searchParams.has('name')) records = records.filter((record) => record.metadata.name === url.searchParams.get('name'));
      if (url.searchParams.has('data_type')) records = records.filter((record) => record.metadata.data_type === url.searchParams.get('data_type'));
      return json(response, 200, { data: records.map((record) => record.metadata) });
    }
    if (request.method === 'GET' && id) {
      const record = state.records.get(id);
      if (!record) return json(response, 404);
      return url.searchParams.get('raw') === 'true'
        ? bytes(response, 200, record.bytes)
        : json(response, 200, record.metadata);
    }
    if (state.config.denyWrites) return json(response, 403);
    if ((request.method === 'POST' && !id) || (request.method === 'PUT' && !id)) {
      const parsed = parseSimpleMultipart(await readBody(request), String(request.headers['content-type'] ?? ''));
      if (!parsed) return json(response, 400);
      const dataId = parsed.id ?? newDataId();
      const old = state.records.get(dataId);
      if (parsed.id && !old) return json(response, 404);
      const item = metadata(dataId, parsed.name ?? old?.metadata.name, parsed.dataType ?? old?.metadata.data_type, parsed.bytes);
      state.records.set(dataId, { metadata: item, bytes: parsed.bytes });
      return json(response, 200, { data: [item] });
    }
    if (request.method === 'DELETE' && id) {
      state.records.delete(id);
      return bytes(response, 200);
    }
    return json(response, 405);
  } catch {
    return json(response, 500, { error: 'synthetic fixture failure' });
  }
}

const dataServer = http.createServer(dataHandler);
const primaryServer = http.createServer(primaryHandler);
dataServer.listen(dataPort, '127.0.0.1', () => {
  const address = dataServer.address();
  dataBase = `http://127.0.0.1:${address.port}`;
  primaryServer.listen(port, '127.0.0.1', () => {
    console.log(`DOMAIN_DATA_FIXTURE_URL=${primaryBase}`);
    console.log(`DOMAIN_DATA_SERVER_URL=${dataBase}`);
  });
});

function shutdown() {
  primaryServer.close(() => dataServer.close(() => process.exit(0)));
}
process.on('SIGINT', shutdown);
process.on('SIGTERM', shutdown);
