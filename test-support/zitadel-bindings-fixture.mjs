// Loopback-only synthetic IdP/API/DDS for Web and Swift adapter tests.
// NOT the Z10 actual-service acceptance harness. No real credentials.
import http from 'node:http';
import { readFile } from 'node:fs/promises';

const port = Number(process.argv[2] ?? 18111);
if (!Number.isInteger(port) || port < 1024 || port > 65535) throw new Error('invalid port');
const base = `http://127.0.0.1:${port}`;
let counts, generation, options;
const reset = (settings = {}) => {
  counts = { refresh: 0, exchange: 0, domains: 0 };
  generation = 0; options = settings;
};
reset();
const server = http.createServer(async (request, response) => {
  response.setHeader('Access-Control-Allow-Origin', '*');
  response.setHeader('Access-Control-Allow-Headers', 'Authorization, Content-Type, Accept, Cache-Control');
  response.setHeader('Access-Control-Allow-Methods', 'GET, POST, OPTIONS');
  response.setHeader('Cache-Control', 'no-store');
  response.setHeader('Content-Type', 'application/json');
  const send = (status, body) => { response.writeHead(status); response.end(JSON.stringify(body)); };
  if (request.method === 'OPTIONS') return send(200, {});
  const url = new URL(request.url, base);
  try {
    // Explicit static allowlist: never serve arbitrary workspace files.
    const files = new Map([
      ['/', ['zitadel-web-host.html', 'text/html']],
      ['/host.js', ['zitadel-web-host.mjs', 'text/javascript']],
      ['/pkg/auki_sdk_web.js', ['../bindings/web/auki-sdk-web/pkg-test/auki_sdk_web.js', 'text/javascript']],
      ['/pkg/auki_sdk_web_bg.wasm', ['../bindings/web/auki-sdk-web/pkg-test/auki_sdk_web_bg.wasm', 'application/wasm']],
    ]);
    if (files.has(url.pathname)) {
      const [file, type] = files.get(url.pathname);
      const bytes = await readFile(new URL(file, import.meta.url));
      response.setHeader('Content-Type', type); response.end(bytes); return;
    }
    let body = '';
    for await (const chunk of request) {
      body += chunk;
      if (body.length > 131072) return send(413, {});
    }
    if (url.pathname === '/__reset' && request.method === 'POST') { reset(body ? JSON.parse(body) : {}); return send(200, {}); }
    if (url.pathname === '/__configure' && request.method === 'POST') { Object.assign(options, JSON.parse(body)); return send(200, {}); }
    if (url.pathname === '/__stats') return send(200, counts);
    if (url.pathname === '/favicon.ico') { response.writeHead(204); return response.end(); }
    if (url.pathname === '/.well-known/openid-configuration') {
      return send(200, { issuer: base, token_endpoint: `${base}/token`, token_endpoint_auth_methods_supported: ['none'] });
    }
    if (url.pathname === '/token' && request.method === 'POST') {
      counts.refresh++;
      const form = new URLSearchParams(body);
      if (options.tokenError) return send(400, { error: options.tokenError, error_description: 'DO_NOT_LEAK_HOST_SECRET' });
      if (form.get('refresh_token') !== `refresh-${generation}` || form.get('client_id') !== 'bindings-client'
          || form.get('grant_type') !== 'refresh_token' || [...form.keys()].length !== 3) return send(400, { error: 'invalid_grant' });
      generation++;
      return send(200, { access_token: `access-${generation}`, refresh_token: `refresh-${generation}`, token_type: 'Bearer', expires_in: 3600 });
    }
    if (url.pathname === '/service/domains-access-token' && request.method === 'POST') {
      counts.exchange++;
      if (options.exchangeFailure) return send(options.exchangeFailure, {});
      if (url.search !== '?purpose=p2p' || request.headers.authorization !== `Bearer access-${generation}`) return send(401, {});
      return send(200, { access_token: 'bindings-dds-bearer' });
    }
    if (url.pathname === '/api/v1/accessible-domains') {
      counts.domains++;
      if (request.headers.authorization !== 'Bearer bindings-dds-bearer') return send(401, {});
      const domains = [{ id: '00000000-0000-0000-0000-000000000001', name: 'Readable Domain', description: '', organization_id: '00000000-0000-0000-0000-000000000002' }];
      return send(200, { domains, total: 1, limit: 100, offset: 0 });
    }
    send(404, {});
  } catch { send(500, {}); }
});
server.listen(port, '127.0.0.1', () => console.log(`Z08 binding fixture at ${base}`));
process.on('SIGINT', () => server.close(() => process.exit(0)));
process.on('SIGTERM', () => server.close(() => process.exit(0)));
