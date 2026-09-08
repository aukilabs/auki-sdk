// Synthetic loopback-only IdP/API/DDS HTTP fixture for browser auth tests.
// This is NOT cross-service acceptance: Z10 runs the actual Auki services.
import http from 'node:http';
import { createHash, createPrivateKey, createPublicKey, randomBytes, sign, verify } from 'node:crypto';

const port = Number(process.argv[2] ?? 18109);
if (!Number.isInteger(port) || port < 1024 || port > 65535) throw new Error('invalid test port');
const counters = () => ({ refresh: 0, exchange: 0, domains: 0, challenge: 0, verify: 0 });
let counts = counters();
let rotated = false;
// The same public test key as browser_auth_tests.rs, so this lifecycle test
// preserves the installed signing-key lineage rather than simulating rotation.
const privateKey = createPrivateKey(`-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQggm4twpf4y/yNNw/k
fqecEEl4zBTwZdRDFUFp/fSxV8qhRANCAARUxrDWJ0AtEGTAYZ4412VPHqMCKoPw
UphDkcOIk7SODsKwUvTIiUr11NbXBJmbBRfhERczsuK4PVha5eg0fVqo
-----END PRIVATE KEY-----`);
const signing = { privateKey, publicKey: createPublicKey(privateKey) };
const publicKey = signing.publicKey.export({ type: 'spki', format: 'pem' });
const keyId = createHash('sha256').update(signing.publicKey.export({ type: 'spki', format: 'der' })).digest('hex');
const proofs = new Map();
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
    counts = counters(); rotated = false; proofs.clear();
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
    return send(500, { error: 'unexpected legacy API exchange' });
  }
  if (url.pathname === '/api/v1/accessible-domains') {
    counts.domains++;
    return send(500, { error: 'unexpected Domain discovery' });
  }
  if (url.pathname === '/service/p2p-verification-keys') {
    return send(200, { version: 1, generation: 1, previous_key_overlap_seconds: 1860,
      keys: [{ id: keyId, status: 'current', signing_method: 'ES256', public_key: publicKey }] });
  }
  const route = /^\/api\/v1\/domains\/([0-9a-f-]{36})\/p2p\/zitadel\/(challenge|verify)$/.exec(url.pathname);
  if (route && request.method === 'POST') {
    const [, domain, operation] = route;
    counts[operation]++;
    if (request.headers.authorization !== 'Bearer browser-new-access') return send(401, {});
    const payload = JSON.parse(body);
    if (operation === 'challenge') {
      const id = randomBytes(16).toString('hex');
      const challenge = randomBytes(32);
      const publicBytes = Buffer.from(payload.public_key, 'base64url');
      if (publicBytes.length !== 36 || publicBytes.subarray(0, 4).toString('hex') !== '08011220') return send(400, {});
      const key = createPublicKey({ key: Buffer.concat([Buffer.from('302a300506032b6570032100', 'hex'), publicBytes.subarray(4)]), type: 'spki', format: 'der' });
      proofs.set(id, { domain, peer: payload.peer_id, challenge, key });
      return send(200, { challenge_id: id, challenge: challenge.toString('base64url'), expires_at: new Date(Date.now() + 60000).toISOString() });
    }
    const proof = proofs.get(payload.challenge_id); proofs.delete(payload.challenge_id);
    if (!proof || proof.domain !== domain || !verify(null, proof.challenge, proof.key, Buffer.from(payload.signature, 'base64url'))) return send(401, {});
    const now = Math.floor(Date.now() / 1000);
    const claims = { type: 'p2p-access', iss: 'dds', aud: ['auki-p2p'], sub: '203845773915743233',
      peer_type: 'user', peer_id: proof.peer, domain_ids: [domain], scopes: ['domain-data:r'], iat: now, exp: now + 1800 };
    const input = [ { alg: 'ES256', typ: 'JWT' }, claims ].map(value => Buffer.from(JSON.stringify(value)).toString('base64url')).join('.');
    const signature = sign('sha256', Buffer.from(input), { key: signing.privateKey, dsaEncoding: 'ieee-p1363' }).toString('base64url');
    return send(200, { peer_id: proof.peer, domain_id: domain, peer_type: 'user', p2p_access_token: `${input}.${signature}`,
      p2p_access_expires_at: new Date(claims.exp * 1000).toISOString() });
  }
  send(404, {});
});
server.listen(port, '127.0.0.1', () => console.log(`ZITADEL browser fixture: http://127.0.0.1:${port}`));
process.on('SIGINT', () => server.close(() => process.exit(0)));
process.on('SIGTERM', () => server.close(() => process.exit(0)));
