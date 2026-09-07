// The ONLY substituted services in Z10: external ZITADEL discovery, rotating
// public-client grants, introspection, and the contract-tested policy /check.
// All Auki API/DDS/DMS/relay requests go to their real implementations.
import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';

export const identityBase = 'http://127.0.0.1:18123';
export function startIdentityFixture(runDir, sdkRoot) {
  const state = { seed: null, grants: new Map(), events: [], stop: false, revoked: false };
  function grant(name) {
    if (!/^[a-z][a-z0-9-]{0,63}$/.test(name)) throw new Error('invalid synthetic grant name');
    if (!state.grants.has(name)) state.grants.set(name, { generation: 0, refreshes: 0, introspections: 0, checks: 0,
      expiresAt: 0, error: null, policy: null, refreshDelayMs: 0 });
    return state.grants.get(name);
  }
  const subject = name => name.startsWith('org-') ? '2938475629384757' : name.startsWith('denied-') ? '2938475629384758' : state.seed.subject;
  const token = (kind, name, generation) => `synthetic-z10-${kind}.${name}.${generation}`;
  const parseToken = (value, kind) => {
    const parts = String(value ?? '').split('.');
    if (parts.length !== 3 || parts[0] !== `synthetic-z10-${kind}`
      || !/^[a-z][a-z0-9-]{0,63}$/.test(parts[1]) || !/^[0-9]+$/.test(parts[2])) return null;
    return { name: parts[1], generation: Number(parts[2]) };
  };
  const server = http.createServer(async (request, response) => {
    response.setHeader('Access-Control-Allow-Origin', '*');
    response.setHeader('Access-Control-Allow-Headers', 'Authorization, Content-Type, Accept, Cache-Control');
    response.setHeader('Access-Control-Allow-Methods', 'GET, POST, OPTIONS');
    response.setHeader('Cache-Control', 'no-store');
    response.setHeader('Content-Type', 'application/json');
    const send = (status, body) => { response.writeHead(status); response.end(JSON.stringify(body)); };
    try {
      if (request.method === 'OPTIONS') return send(200, {});
      const url = new URL(request.url, identityBase);
      let body = '';
      for await (const chunk of request) { body += chunk; if (body.length > 131072) return send(413, {}); }
      if (url.pathname === '/__seed' && request.method === 'POST') {
        if (state.seed) return send(409, {});
        state.seed = JSON.parse(body); return send(200, {});
      }
      if (url.pathname === '/__config') return send(200, { ...state.seed, stop: state.stop, revoked: state.revoked });
      if (url.pathname === '/__stats') return send(200, {
        grants: Object.fromEntries(state.grants), events: state.events, stop: state.stop, revoked: state.revoked,
      });
      if (url.pathname === '/__event' && request.method === 'POST') {
        const event = JSON.parse(body);
        state.events.push({ ...event, receivedAt: new Date().toISOString() });
        fs.appendFileSync(path.join(runDir, 'events.jsonl'), JSON.stringify(state.events.at(-1))+'\n', { mode: 0o600 });
        return send(200, {});
      }
      if (url.pathname === '/__configure' && request.method === 'POST') {
        const config = JSON.parse(body);
        if (typeof config.stop === 'boolean') state.stop = config.stop;
        if (typeof config.revoked === 'boolean') state.revoked = config.revoked;
        if (config.grant) {
          const current = grant(config.grant);
          for (const key of ['error', 'policy', 'refreshDelayMs']) if (key in config) current[key] = config[key];
        }
        return send(200, {});
      }
      if (url.pathname.startsWith('/__credentials/')) {
        const name = url.pathname.slice('/__credentials/'.length); grant(name);
        return send(200, { accessToken: token('access', name, 0), refreshToken: token('refresh', name, 0),
          clientId: `z10-${name}`, issuer: identityBase, accessTokenExpiresAt: '2020-01-01T00:00:00Z' });
      }
      if (url.pathname === '/.well-known/openid-configuration') return send(200, {
        issuer: identityBase, token_endpoint: `${identityBase}/token`, introspection_endpoint: `${identityBase}/introspect`,
        token_endpoint_auth_methods_supported: ['none', 'private_key_jwt'], grant_types_supported: ['refresh_token'],
      });
      if (url.pathname === '/token' && request.method === 'POST') {
        const form = new URLSearchParams(body), parsed = parseToken(form.get('refresh_token'), 'refresh');
        if (!parsed || !state.grants.has(parsed.name)) return send(400, { error: 'invalid_grant' });
        const g = grant(parsed.name); g.refreshes++;
        if (g.error) return send(400, { error: g.error });
        if (form.get('client_id') !== `z10-${parsed.name}` || form.get('grant_type') !== 'refresh_token'
          || [...form.keys()].length !== 3 || parsed.generation !== g.generation) return send(400, { error: 'invalid_grant' });
        g.generation++; g.expiresAt = Math.floor(Date.now()/1000)+3600;
        if (g.refreshDelayMs) await new Promise(resolve => setTimeout(resolve, g.refreshDelayMs));
        return send(200, { access_token: token('access', parsed.name, g.generation),
          refresh_token: token('refresh', parsed.name, g.generation), expires_in: 3600, token_type: 'Bearer' });
      }
      if (url.pathname === '/introspect' && request.method === 'POST') {
        const parsed = parseToken(new URLSearchParams(body).get('token'), 'access');
        if (!parsed || !state.grants.has(parsed.name)) return send(200, { active: false });
        const g = grant(parsed.name); g.introspections++;
        if (parsed.generation !== g.generation || g.expiresAt <= Date.now()/1000) return send(200, { active: false });
        return send(200, { active: true, iss: identityBase, sub: subject(parsed.name), aud: ['z10-api'],
          exp: g.expiresAt, iat: g.expiresAt-3600, client_id: `z10-${parsed.name}`, scope: 'openid profile email',
          'urn:zitadel:iam:user:metadata': {
            'auki-api-organization': Buffer.from(state.seed.organizationId).toString('base64').replace(/=+$/, ''),
            'auki-domain-permissions': Buffer.from('[]').toString('base64').replace(/=+$/, ''),
          } });
      }
      if (url.pathname === `/orgs/${state.seed?.organizationId}/authorization/check` && request.method === 'POST') {
        const parsed = parseToken(request.headers.authorization?.replace(/^Bearer /, ''), 'access');
        if (!parsed || !state.grants.has(parsed.name)) return send(401, {});
        const g = grant(parsed.name); g.checks++;
        if (parsed.generation !== g.generation || g.expiresAt <= Date.now()/1000) return send(401, {});
        const input = JSON.parse(body);
        if (input.sub !== subject(parsed.name) || input.permission !== 'domain_metadata_read') return send(400, {});
        if (g.policy === 'outage') return send(503, {});
        const candidate = [state.seed.domainId, state.seed.otherDomainId].includes(input.domain_id);
        const allowed = candidate && !state.revoked && g.policy !== 'deny' && !parsed.name.startsWith('denied-')
          && (parsed.name.startsWith('org-') || input.domain_id === state.seed.domainId);
        return send(200, { allowed });
      }
      return send(404, {}); // In particular there is NO legacy /domains fixture.
    } catch { if (!response.headersSent) send(500, { error: 'synthetic fixture failure' }); else response.end(); }
  });
  const host = http.createServer((request, response) => {
    const pathname = new URL(request.url, 'http://127.0.0.1:18130').pathname;
    const files = {
      '/': ['test-support/zitadel-local-web.html', 'text/html'],
      '/host.mjs': ['test-support/zitadel-local-web.mjs', 'text/javascript'],
      '/pkg/auki_sdk_web.js': ['bindings/web/auki-sdk-web/pkg-test/auki_sdk_web.js', 'text/javascript'],
      '/pkg/auki_sdk_web_bg.wasm': ['bindings/web/auki-sdk-web/pkg-test/auki_sdk_web_bg.wasm', 'application/wasm'],
    };
    const file = files[pathname];
    if (!file) { response.writeHead(404); response.end(); return; }
    response.setHeader('Content-Type', file[1]); response.setHeader('Cache-Control', 'no-store');
    fs.createReadStream(path.join(sdkRoot, file[0])).on('error', () => { response.destroy(); }).pipe(response);
  });
  server.listen(18123, '127.0.0.1'); host.listen(18130, '127.0.0.1');
  return { state, server, host };
}
