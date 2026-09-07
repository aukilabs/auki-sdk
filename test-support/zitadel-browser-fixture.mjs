// Synthetic loopback-only IdP/API/DDS HTTP fixture for browser auth tests.
// This is NOT cross-service acceptance: Z10 runs the actual Auki services.
import http from 'node:http';

const port = Number(process.argv[2] ?? 18109);
if (!Number.isInteger(port) || port < 1024 || port > 65535) throw new Error('invalid test port');
let counts = { refresh: 0, exchange: 0, domains: 0 };
let rotated = false;
const server = http.createServer(async (request, response) => {
  response.setHeader('Access-Control-Allow-Origin', '*');
  response.setHeader('Access-Control-Allow-Headers', 'Authorization, Content-Type, Accept, Cache-Control');
  response.setHeader('Access-Control-Allow-Methods', 'GET, POST, OPTIONS');
  response.setHeader('Cache-Control', 'no-store');
  response.setHeader('Content-Type', 'application/json');
  const send = (status, body) => { response.writeHead(status); response.end(JSON.stringify(body)); };
  if (request.method === 'OPTIONS') return send(200, {});
  const base = `http://127.0.0.1:${port}`;
  const url = new URL(request.url, base);
  let body = '';
  for await (const chunk of request) {
    body += chunk;
    if (body.length > 131072) return send(413, {});
  }
  if (url.pathname === '/__reset' && request.method === 'POST') {
    counts = { refresh: 0, exchange: 0, domains: 0 }; rotated = false;
    return send(200, {});
  }
  if (url.pathname === '/__stats') return send(200, counts);
  if (url.pathname === '/.well-known/openid-configuration') {
    return send(200, { issuer: base, token_endpoint: `${base}/token`, token_endpoint_auth_methods_supported: ['none'] });
  }
  if (url.pathname === '/token' && request.method === 'POST') {
    counts.refresh++;
    const form = new URLSearchParams(body);
    if (rotated || form.get('refresh_token') !== 'browser-old-refresh' || form.get('client_id') !== 'browser-client'
        || form.get('grant_type') !== 'refresh_token' || [...form.keys()].length !== 3) {
      return send(400, { error: 'invalid_grant' });
    }
    rotated = true;
    // Leave a deterministic window to cancel the browser caller after the
    // server consumes the old grant, before it returns the replacement.
    await new Promise(resolve => setTimeout(resolve, 150));
    return send(200, { access_token: 'browser-new-access', refresh_token: 'browser-new-refresh', token_type: 'Bearer', expires_in: 3600 });
  }
  if (url.pathname === '/service/domains-access-token' && request.method === 'POST') {
    counts.exchange++;
    if (url.search !== '?purpose=p2p' || request.headers.authorization !== 'Bearer browser-new-access') return send(401, {});
    return send(200, { access_token: 'browser-dds-bearer' });
  }
  if (url.pathname === '/api/v1/accessible-domains') {
    counts.domains++;
    if (request.headers.authorization !== 'Bearer browser-dds-bearer') return send(401, {});
    return send(200, { domains: [], total: 0, limit: 100, offset: 0 });
  }
  send(404, {});
});
server.listen(port, '127.0.0.1', () => console.log(`ZITADEL browser fixture: http://127.0.0.1:${port}`));
process.on('SIGINT', () => server.close(() => process.exit(0)));
process.on('SIGTERM', () => server.close(() => process.exit(0)));
