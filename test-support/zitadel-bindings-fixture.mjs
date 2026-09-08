// Loopback-only synthetic IdP/API/DDS for Web and Swift adapter tests.
// NOT the Z10 actual-service acceptance harness. No real credentials.
import http from 'node:http';
import { readFile } from 'node:fs/promises';

const port = Number(process.argv[2] ?? 18111);
if (!Number.isInteger(port) || port < 1024 || port > 65535) throw new Error('invalid port');
const base = `http://127.0.0.1:${port}`;
let counts, generation, options;
let phase = { phase: 'idle' };
const reset = (settings = {}) => {
  counts = { refresh: 0, exchange: 0, domains: 0, admission: 0 };
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
    // Z09 real Expo host coordination; synthetic diagnostics only.
    if (url.pathname === '/__phase') {
      if (request.method === 'POST') phase = JSON.parse(body);
      return send(200, phase);
    }
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
      return send(500, { error: 'unexpected legacy API exchange' });
    }
    if (url.pathname === '/api/v1/accessible-domains') {
      counts.domains++;
      return send(500, { error: 'unexpected Domain discovery' });
    }
    if (/^\/api\/v1\/domains\/00000000-0000-0000-0000-000000000099\/p2p\/zitadel\/challenge$/.test(url.pathname) && request.method === 'POST') {
      counts.admission++;
      if (options.admissionFailure) return send(options.admissionFailure, {});
      if (request.headers.authorization !== `Bearer access-${generation}`) return send(401, {});
      // Binding lifecycle probes deliberately stop at a denied Domain. They
      // prove refresh/ACK/cancellation across FFI, not successful P2P admission.
      // The isolated actual-service harness separately proves successful peers.
      return send(403, {});
    }
    send(404, {});
  } catch { send(500, {}); }
});
server.listen(port, '127.0.0.1', () => console.log(`Z08 binding fixture at ${base}`));
process.on('SIGINT', () => server.close(() => process.exit(0)));
process.on('SIGTERM', () => server.close(() => process.exit(0)));
