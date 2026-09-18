// Contract composition: M1 data fixture + test_tasks.py/test_robot_tasks.py,
// auki-auth tests, discovery.rs and relay-booking tests. All keys are ephemeral.
import http from 'node:http';
import { createHash, createPublicKey, generateKeyPairSync, randomBytes, randomUUID, sign, verify } from 'node:crypto';
import { startFixture, DOMAIN } from './fixture.mjs';
export const HOST = '127.0.0.1.sslip.io';
const ROBOT = 'dddddddd-dddd-4ddd-8ddd-dddddddddddd';
const exp = seconds => new Date(Date.now() + seconds * 1000).toISOString();
const b64 = value => Buffer.from(JSON.stringify(value)).toString('base64url');
function base58(bytes) {
  const alphabet = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz';
  let n = BigInt('0x' + bytes.toString('hex')), out = '';
  while (n) { out = alphabet[Number(n % 58n)] + out; n /= 58n; }
  for (const byte of bytes) { if (byte) break; out = '1' + out; }
  return out;
}
export async function startNetworkFixture(relay, wssPort) {
  const m1 = await startFixture();
  const signing = generateKeyPairSync('ec', { namedCurve: 'prime256v1' });
  const publicKey = signing.publicKey.export({ type: 'spki', format: 'pem' });
  const keyId = createHash('sha256').update(signing.publicKey.export({ type: 'spki', format: 'der' })).digest('hex');
  const jwt = claims => {
    const input = b64({ alg: 'ES256', typ: 'JWT' }) + '.' + b64(claims);
    return input + '.' + sign('sha256', Buffer.from(input), { key: signing.privateKey, dsaEncoding: 'ieee-p1363' }).toString('base64url');
  };
  const proofs = new Map(), issued = new Map(), advertisements = new Map(), bookings = new Map();
  const registration = randomBytes(24).toString('hex');
  const state = { claims: 0, requests: [], denyP2p: false, verifiedProofs: 0, withdrawn: 0, released: 0, renewed: 0, proofDelay: 0, discoveryDelay: 0, chatDiscoveryMode: 'normal', chatDiscoveryGate: undefined, pendingProofs: 0, pendingDiscovery: 0 };
  let base;
  const issue = claims => { const token = jwt(claims); issued.set(token, claims); return token; };
  const machine = peer => {
    const now = Math.floor(Date.now()/1000);
    const claims = { iss: 'dds', aud: [base + '/robots'], sub: ROBOT, node_id: ROBOT,
    organization_id: DOMAIN, node_type: 'robot', node_mode: 'dedicated', assigned_domain_id: DOMAIN,
    ...(peer ? { peer_id: peer } : {}), iat: now-1, exp: now+1800 };
    return { access_token: issue(claims), access_expires_at: new Date(claims.exp*1000).toISOString() };
  };
  const grant = (peer, robot = false) => {
    const issuedAt = Math.floor(Date.now()/1000)-1;
    const claims = { type: 'p2p-access', iss: 'dds', aud: ['auki-p2p'], sub: robot ? ROBOT : 'fixture-user',
      peer_type: robot ? 'robot' : 'user', peer_id: peer, domain_ids: [DOMAIN], scopes: ['domain-data:r'],
      iat: issuedAt, exp: issuedAt+1800 };
    return { peer_id: peer, domain_id: DOMAIN, peer_type: claims.peer_type,
      p2p_access_token: issue(claims), p2p_access_expires_at: new Date(claims.exp*1000).toISOString() };
  };
  const server = http.createServer(async (req, res) => {
    res.setHeader('Access-Control-Allow-Origin', '*');
    res.setHeader('Access-Control-Allow-Headers', 'Authorization, Content-Type, Accept, Cache-Control, posemesh-client-id, posemesh-sdk-version');
    res.setHeader('Access-Control-Allow-Methods', 'GET, POST, PUT, DELETE, OPTIONS');
    res.setHeader('Access-Control-Expose-Headers', 'Location');
    res.setHeader('Cache-Control', 'no-store');
    const send = (status, value) => { res.writeHead(status, status === 204 ? {} : { 'Content-Type': 'application/json' }); res.end(value === undefined ? undefined : JSON.stringify(value)); };
    if (req.method === 'OPTIONS') return send(200, {});
    const url = new URL(req.url, base), path = url.pathname;
    state.requests.push(req.method + ' ' + path);
    let raw = '';
    try {
      for await (const chunk of req) { raw += chunk; if (raw.length > 65536) return send(413, {}); }
      const body = raw ? JSON.parse(raw) : {};
      const auth = req.headers.authorization?.replace(/^Bearer /, '');
      const principal = issued.get(auth);
      if (path === '/service/p2p-verification-keys') return send(200, { version: 1, generation: 1,
        previous_key_overlap_seconds: 1860, keys: [{ id: keyId, status: 'current', signing_method: 'ES256', public_key: publicKey }] });
      if (path === '/internal/v1/robots/register' || path === '/internal/v1/auth/robot/verify') {
        if (body.registration_credentials !== registration) return send(401, {});
        return send(200, { robot_id: ROBOT, ...machine() });
      }
      const robotProof = path.startsWith('/internal/v1/auth/p2p/');
      const userProof = path.startsWith(`/api/v1/domains/${DOMAIN}/p2p/`) && /\/(challenge|verify)$/.test(path);
      if (robotProof || userProof) {
        if (state.denyP2p || (robotProof ? principal?.node_type !== 'robot' : auth !== 'synthetic-service')) return send(403, {});
        if (path.endsWith('/challenge')) {
          if (userProof && state.proofDelay) { state.pendingProofs++; await new Promise(r => setTimeout(r, state.proofDelay)); state.pendingProofs--; }
          const bytes = Buffer.from(body.public_key, 'base64url');
          if (bytes.length !== 36 || bytes.subarray(0,4).toString('hex') !== '08011220'
              || base58(Buffer.concat([Buffer.from([0,36]), bytes])) !== body.peer_id) return send(400, {});
          const key = createPublicKey({ key: Buffer.concat([Buffer.from('302a300506032b6570032100','hex'), bytes.subarray(4)]), type: 'spki', format: 'der' });
          const id = randomUUID(), challenge = randomBytes(32);
          if (proofs.size >= 64) return send(429, {});
          proofs.set(id, { key, challenge, peer: body.peer_id, robot: robotProof, expires: Date.now()+60000 });
          return send(200, { challenge_id: id, challenge: challenge.toString('base64url'), expires_at: exp(60) });
        }
        const proof = proofs.get(body.challenge_id); proofs.delete(body.challenge_id);
        if (!proof || proof.robot !== robotProof || proof.expires < Date.now()
            || !verify(null, proof.challenge, proof.key, Buffer.from(body.signature, 'base64url'))) return send(401, {});
        state.verifiedProofs++;
        return send(200, robotProof ? { peer_id: proof.peer, ...machine(proof.peer) } : grant(proof.peer));
      }
      if (path === '/internal/v1/auth/robot/p2p-token') {
        if (!principal?.peer_id || principal.node_type !== 'robot' || body.domain_id !== DOMAIN) return send(403, {});
        return send(200, grant(principal.peer_id, true));
      }
      if (path === '/tasks' || path.startsWith('/tasks/')) { state.claims++; return send(403, {}); }
      if (path.includes('/p2p/advertisements')) {
        if (principal?.type !== 'p2p-access' || !principal.domain_ids.includes(DOMAIN) || !path.startsWith(`/api/v1/domains/${DOMAIN}/`)) return send(403, {});
        if (req.method === 'PUT') {
          const ad = { ...body, peer_id: principal.peer_id, subject_id: principal.sub, peer_type: principal.peer_type, expires_at: exp(120) };
          advertisements.set(principal.peer_id, ad); return send(200, ad);
        }
        if (req.method === 'DELETE') { advertisements.delete(principal.peer_id); state.withdrawn++; return send(204); }
        if (state.discoveryDelay) { state.pendingDiscovery++; await new Promise(r => setTimeout(r, state.discoveryDelay)); state.pendingDiscovery--; }
        if (url.searchParams.get('protocol') === '/example/core-explorer-chat/1.0.0') {
          if (state.chatDiscoveryGate) {
            state.pendingDiscovery++;
            try { await state.chatDiscoveryGate(); } finally { state.pendingDiscovery--; }
          }
          if (state.chatDiscoveryMode === 'denied') return send(403, {});
          if (state.chatDiscoveryMode === 'empty') return send(200, { advertisements: [] });
          if (state.chatDiscoveryMode === 'malformed') return send(200, { advertisements: 'invalid' });
          if (state.chatDiscoveryMode === 'offline') return req.socket.destroy();
        }
        return send(200, { advertisements: [...advertisements.values()].filter(ad => Date.parse(ad.expires_at) > Date.now()).filter(ad => !url.searchParams.has('protocol') || ad.protocols.includes(url.searchParams.get('protocol'))) });
      }
      if (path.startsWith('/relay-bookings')) {
        if (!principal?.peer_id) return send(403, {});
        const peer = principal.peer_id;
        if (req.method === 'DELETE') { bookings.delete(peer); state.released++; return send(204); }
        if (path.endsWith('/renew') && req.method === 'POST' && bookings.has(peer)) {
          const snapshot = bookings.get(peer);
          snapshot.authority_expires_at = exp(300);
          for (const slot of snapshot.slots) slot.provider_lease_expires_at = exp(180);
          state.renewed++;
          return send(200, snapshot);
        }
        if (path.endsWith('/active')) return bookings.has(peer) ? send(200, bookings.get(peer)) : send(204);
        if (req.method === 'POST' && path === '/relay-bookings') {
          const id = randomUUID();
          const snapshot = { booking_id: id, mode: 'public', state: 'active', relay_count: 1,
            requested_duration_seconds: body.requested_duration_seconds ?? 86400, requested_until: exp(86400), authority_expires_at: exp(300),
            assigned_count: 1, provider_ready_count: 1, unfilled_count: 0, created_at: exp(0), slots: [{
              slot_id: randomUUID(), slot_index: 0, state: 'ready', assignment_id: randomUUID(), reservation_epoch: randomUUID(),
              provider_peer_id: relay.peer, provider_base_addresses: [
                `/dns4/${HOST}/tcp/${relay.port}/p2p/${relay.peer}`,
                `/dns4/${HOST}/tcp/${wssPort}/wss/p2p/${relay.peer}`],
              limits: { duration_seconds: 120, data_bytes_per_direction: 10485760 }, provider_lease_expires_at: exp(180) }] };
          bookings.set(peer, snapshot); res.setHeader('Location', `/relay-bookings/${id}`); return send(201, snapshot);
        }
        return bookings.has(peer) ? send(200, bookings.get(peer)) : send(404, {});
      }
      // M1 owns login and Domain data; proxy only to its fixed loopback origin.
      const upstream = await fetch(m1.base + req.url, { method: req.method, headers: { 'Content-Type': 'application/json', ...(auth ? { Authorization: 'Bearer ' + auth } : {}) }, body: ['GET','HEAD'].includes(req.method) ? undefined : raw, signal: AbortSignal.timeout(5000) });
      res.writeHead(upstream.status, { 'Content-Type': upstream.headers.get('content-type') || 'application/json' });
      res.end(Buffer.from(await upstream.arrayBuffer()));
    } catch { if (!res.headersSent) send(500, {}); else res.end(); }
  });
  server.requestTimeout = 10000;
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  base = `http://127.0.0.1:${server.address().port}`;
  return { base, registration, state, advertisements, bookings, secrets: () => [...issued.keys()], close: async () => {
    proofs.clear(); issued.clear(); advertisements.clear(); bookings.clear(); server.closeAllConnections();
    await new Promise(resolve => server.close(resolve)); await m1.close();
  } };
}
