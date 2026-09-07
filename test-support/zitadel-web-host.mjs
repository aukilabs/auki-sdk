import init, { AukiUserSession } from '/pkg/auki_sdk_web.js';

const assert = (condition, message) => { if (!condition) throw new Error(message); };
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const barrier = () => { let resolve, reject; const promise = new Promise((a, b) => { resolve = a; reject = b; }); return { promise, resolve, reject }; };
const api = async (path, body) => (await fetch(path, body === undefined ? {} : { method: 'POST', body: JSON.stringify(body) })).json();
const payload = () => ({ accessToken: 'access-0', refreshToken: 'refresh-0', clientId: 'bindings-client', issuer: location.origin, accessTokenExpiresAt: '2020-01-01T07:00:00.123456789+07:00' });
const snapshot = c => ({ accessToken: c.exposeAccessToken(), refreshToken: c.exposeRefreshToken(), clientId: c.clientId, issuer: c.issuer, accessTokenExpiresAt: c.accessTokenExpiresAt });
const session = (store, credentials = payload()) => AukiUserSession.importZitadelWithEnvironment(location.origin, location.origin, location.origin, credentials, store);
const errorCode = async (operation, code) => {
  try { await operation(); throw new Error(`expected ${code}`); }
  catch (error) {
    assert(error.code === code && error.name === 'AukiAuthError', `wrong auth error: ${error}`);
    assert(!String(error).includes('DO_NOT_LEAK'), 'leaked host/provider diagnostic');
  }
};

export async function run() {
  await init();
  const passed = [];
  const test = async (name, fn) => { await fn(); passed.push(name); document.querySelector('#result').textContent = passed.join('\n'); };
  await test('synchronous import, acknowledged save, concurrent waiters, restart', async () => {
    await api('/__reset', {});
    const entered = barrier(), release = barrier(); let saves = 0, durable;
    const s = session(async c => {
      saves++; durable = snapshot(c);
      assert(JSON.stringify(c).includes('redacted') && !JSON.stringify(c).includes('access-1'), 'snapshot logging must redact');
      assert(String(c).includes('redacted'), 'snapshot toString must redact');
      assert(durable.accessTokenExpiresAt.endsWith('Z'), 'expiry must be UTC');
      entered.resolve(); await release.promise;
    });
    assert(!(s instanceof Promise) && (await api('/__stats')).refresh === 0, 'import must not perform I/O');
    const calls = [s.accessibleDomains(), s.accessibleDomains()];
    await entered.promise;
    assert((await api('/__stats')).exchange === 0, 'must not exchange before durable ACK');
    release.resolve();
    const domains = await Promise.all(calls);
    assert(domains.every(d => d.length === 1 && d[0].name === 'Readable Domain'), 'Domain result mismatch');
    assert(saves === 1 && (await api('/__stats')).refresh === 1, 'refresh/save must be single flight');
    await errorCode(() => s.startPeer('00000000-0000-0000-0000-000000000099'), 'authorization_denied');
    await s.close();
    const restarted = session(async () => { throw new Error('unexpected refresh'); }, durable);
    assert((await restarted.accessibleDomains()).length === 1, 'saved credentials must restart');
    assert((await api('/__stats')).refresh === 1, 'restart must reuse saved generation');
    await restarted.close();
  });
  await test('rejected/throwing/unacknowledged save retains generation for retry', async () => {
    for (const failure of ['reject', 'throw', 'undefined']) {
      await api('/__reset', {}); let fail = true, saves = 0; const generations = [];
      const s = session(c => {
        saves++; generations.push(c.exposeRefreshToken());
        if (fail) {
          if (failure === 'throw') throw new Error('DO_NOT_LEAK_HOST_SECRET');
          if (failure === 'undefined') return undefined;
          return Promise.reject(new Error('DO_NOT_LEAK_HOST_SECRET'));
        }
        return Promise.resolve();
      });
      await errorCode(() => s.accessibleDomains(), 'persistence');
      assert((await api('/__stats')).exchange === 0, 'save failure must fence exchange');
      fail = false;
      assert((await s.accessibleDomains()).length === 1, 'same handle must recover');
      assert(saves === 2 && generations.join(',') === 'refresh-1,refresh-1', 'retry must save the replacement, not rotate again');
      assert((await api('/__stats')).refresh === 1, 'save retry replayed refresh');
      await s.close();
    }
  });
  await test('startup exchange failure keeps imported handle recoverable', async () => {
    await api('/__reset', { exchangeFailure: 503 }); let saves = 0;
    const s = session(async () => { saves++; });
    await errorCode(() => s.accessibleDomains(), 'transient');
    await api('/__configure', { exchangeFailure: null });
    assert((await s.accessibleDomains()).length === 1 && saves === 1, 'startup retry must keep rotated credentials');
    await s.close();
  });
  await test('close drains pending save, fences waiters, prevents late writes', async () => {
    await api('/__reset', {});
    const entered = barrier(), release = barrier(); let writes = 0, closed = false;
    const s = session(async () => { entered.resolve(); await release.promise; writes++; });
    const operation = errorCode(() => s.accessibleDomains(), 'closed');
    await entered.promise;
    const closing = s.close().then(() => { closed = true; });
    await delay(100);
    assert(!closed && writes === 0, 'close must drain the host callback');
    release.resolve(); await Promise.all([closing, operation]);
    assert(writes === 1 && (await api('/__stats')).exchange === 0, 'close must prevent downstream I/O');
    writes = 0; // host clearing storage happens only now
    await errorCode(() => s.accessibleDomains(), 'closed');
    await s.close(); await delay(50); assert(writes === 0, 'late save after completed close');
  });
  await test('typed terminal auth failures and redacted validation', async () => {
    for (const [tokenError, code] of [['invalid_grant', 'authentication_required'], ['invalid_client', 'configuration']]) {
      await api('/__reset', { tokenError });
      const s = session(async () => { throw new Error('must not save'); });
      await errorCode(() => s.accessibleDomains(), code);
      await errorCode(() => s.accessibleDomains(), code);
      assert((await api('/__stats')).refresh === 1, 'terminal error must latch');
      await s.close();
    }
    await errorCode(() => session(async () => {}, { ...payload(), issuer: 'DO_NOT_LEAK_HOST_SECRET' }), 'configuration');
    await errorCode(() => session(async () => {}, { ...payload(), accessTokenExpiresAt: 'DO_NOT_LEAK_HOST_SECRET' }), 'configuration');
    await api('/__reset', { exchangeFailure: 403 });
    const s = session(async () => {});
    await errorCode(() => s.accessibleDomains(), 'authorization_denied'); await s.close();
  });
  document.querySelector('#result').textContent = `PASS ${passed.length} Web binding cases\n${passed.join('\n')}`;
  return passed;
}
globalThis.__z08 = run().then(result => ({ result }), error => {
  document.querySelector('#result').textContent = `FAIL ${error.stack}`; return { error: String(error) };
});
