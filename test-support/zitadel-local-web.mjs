import init, { AukiUserSession, AukiDiscoveryMode, AukiPeerReachabilityMode, AukiInfoClient, AukiInfoEndpoint } from './pkg/auki_sdk_web.js';
const fixture = 'http://127.0.0.1:18123';
const api = 'http://127.0.0.1:18120', dds = 'http://127.0.0.1:18121', dms = 'http://127.0.0.1:18122/v1/';
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const check = (ok, why) => { if (!ok) throw new Error(why); };
async function request(path, body) {
  const response = await fetch(fixture+path, body === undefined ? {} : { method:'POST', headers:{'Content-Type':'application/json'}, body:JSON.stringify(body) });
  check(response.ok, 'fixture request failed'); return response.json();
}
async function event(value) { await request('/__event', { runtime:'browser', ...value }); }
function show(value) { document.querySelector('#result').textContent = value; }
function extract(c) { return { accessToken:c.exposeAccessToken(), refreshToken:c.exposeRefreshToken(), clientId:c.clientId, issuer:c.issuer, accessTokenExpiresAt:c.accessTokenExpiresAt }; }
async function storage(key, value) {
  const db = await new Promise((resolve, reject) => {
    const open = indexedDB.open('zitadel-z10-synthetic', 1);
    open.onupgradeneeded = () => open.result.createObjectStore('credentials');
    open.onsuccess = () => resolve(open.result); open.onerror = () => reject(Error('storage failed'));
  });
  try { await new Promise((resolve, reject) => {
    const tx = db.transaction('credentials', 'readwrite', { durability:'strict' });
    if (value === null) tx.objectStore('credentials').delete(key); else tx.objectStore('credentials').put(JSON.stringify(value), key);
    tx.oncomplete = resolve; tx.onabort = tx.onerror = () => reject(Error('storage failed'));
  }); } finally { db.close(); }
}
async function imported(name, save) {
  return AukiUserSession.importZitadelWithEnvironment(api, dds, dms, await request(`/__credentials/${name}`), save);
}
async function fails(code, operation) {
  try { await operation(); throw new Error('expected rejection'); }
  catch (error) { check(error.code === code, `expected ${code}, received ${error.code ?? 'untyped failure'}`); }
}

async function admit(session, domain) {
  const peer = await session.startPeer(domain, AukiPeerReachabilityMode.OutboundOnly);
  await peer.shutdown(); peer.free();
}

async function run() {
  await init();
  const seed = await request('/__config');
  let rejectSave = true, replacement, saves = 0;
  const recovery = await imported('browser-save', async c => {
    saves++; replacement = extract(c); c.free();
    if (rejectSave) throw Error('synthetic storage failure');
    await storage('recovery', replacement);
  });
  await fails('persistence', () => admit(recovery, seed.domainId));
  rejectSave = false;
  await admit(recovery, seed.domainId);
  check(saves === 2 && (await request('/__stats')).grants['browser-save'].refreshes === 1, 'save retry replayed refresh');
  await recovery.close(); recovery.free();
  const restarted = AukiUserSession.importZitadelWithEnvironment(api, dds, dms, replacement, async () => { throw Error('unexpected rotation'); });
  await admit(restarted, seed.domainId);
  await fails('authorization_denied', () => restarted.startPeer(seed.otherDomainId));
  await restarted.close(); restarted.free(); await storage('recovery', null);
  const invalid = await imported('browser-invalid', async () => { throw Error('must not save'); });
  await request('/__configure', {grant:'browser-invalid', error:'invalid_grant'});
  await fails('authentication_required', () => admit(invalid, seed.domainId));
  await fails('authentication_required', () => admit(invalid, seed.domainId));
  check((await request('/__stats')).grants['browser-invalid'].refreshes === 1, 'terminal failure retried');
  await invalid.close(); invalid.free();
  await event({event:'recovery-passed', cases:4});

  let liveSaves = 0, terminal;
  const session = await imported('browser-live', async c => {
    liveSaves++; const value = extract(c); c.free(); await storage('live', value);
  });
  const peers = await Promise.all([0, 1].map(() => session.startPeerWithDiscovery(seed.domainId, AukiDiscoveryMode.DiscoverAndAdvertise)));
  const ids = peers.map(p => p.peerId);
  const endpoints = peers.map(peer => AukiInfoEndpoint.mount(peer, requester => ({
    app:'zitadel-acceptance', appVersion:'0.1.0', name:'browser', sessionId:requester.subject,
    sessionClockId:'z10', sessionClockHash:'z10', sessionNowNs:0, peerId:peer.peerId, appInstance:'browser',
  })));
  const clients = peers.map(peer => new AukiInfoClient(peer));
  for (const peer of peers) void peer.waitStopped().catch(error => { terminal = error; });
  check((await request('/__stats')).grants['browser-live'].refreshes === 1, 'concurrent start replayed refresh');
  await event({ event:'ready', peerIds:ids });
  const started = Date.now(); let successes = 0, checkedRevocation = false;
  try {
    for (;;) {
      const config = await request('/__config');
      if (config.stop) break;
      if (terminal) throw terminal;
      check(Date.now()-started < 95*60000, 'browser acceptance deadline');
      let natives = 0;
      for (let i=0; i<peers.length; i++) {
        const candidates = await peers[i].discoverProtocol(clients[i].protocol);
        try {
          for (const candidate of candidates) {
            check(candidate.subjectId === seed.subject, 'discovery changed opaque subject');
            const route = candidate.routes.find(route => route.includes('/wss/') && route.includes('/p2p-circuit/'));
            if (!route) continue;
            let info;
            try { info = await clients[i].fetchExact({ peerId:candidate.peerId, route }); }
            catch { continue; } // controller bounds the last successful probe
            check(info.sessionId === seed.subject, 'remote authentication changed opaque subject');
            successes++; if (info.name === 'native') natives++;
          }
        } finally { for (const candidate of candidates) candidate.free(); }
      }
      check(JSON.stringify(peers.map(p => p.peerId)) === JSON.stringify(ids), 'browser Peer ID changed');
      if (config.revoked && !checkedRevocation) {
        const fresh = await imported('browser-revoked', async c => {
          const value = extract(c); c.free(); await storage('revoked', value);
        });
        await fails('authorization_denied', () => admit(fresh, seed.domainId));
        await fresh.close(); fresh.free(); await storage('revoked', null);
        await event({event:'revocation-checked', retainedPeerIds:ids, nativeProbes:natives});
        checkedRevocation = true;
      }
      await event({event:'probe', peerIds:ids, successes, nativeProbes:natives, saves:liveSaves});
      show(`RUNNING: 2 browser peers, ${successes} authenticated relay probes, ${liveSaves} durable saves\nPeer IDs unchanged\n${ids.join('\n')}`);
      await delay(10000);
    }
  } catch (error) {
    // Preserve the primary failure even if shutdown reports a second error.
    await event({event:'failed', phase:'run', reason:error.message});
    throw error;
  } finally {
    for (const endpoint of endpoints) { await endpoint.close(); endpoint.free(); }
    for (const client of clients) client.free();
    for (const peer of peers) { await peer.shutdown(); peer.free(); }
    await session.close(); session.free(); await storage('live', null);
  }
  await event({event:'stopped', successes}); show(`PASS browser local acceptance: ${successes} authenticated relay probes`);
}
run().catch(async error => {
  show(`FAIL ${error.message}`);
  await event({event:'failed', reason:error.message});
});
