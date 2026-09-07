import Auki, { importZitadelSession, closeSession } from '@aukilabs/auki-sdk-expo';
import { AppState, Platform } from 'react-native';
import { durable } from './storage';

export const base = 'http://127.0.0.1:18111';
const environment = { apiBaseUrl: base, ddsBaseUrl: base, dmsBaseUrl: base };
const credentials = () => ({ accessToken: 'access-0', refreshToken: 'refresh-0', clientId: 'bindings-client', issuer: base, accessTokenExpiresAt: '2020-01-01T00:00:00.123456789Z' });
const extract = c => ({ accessToken: c.exposeAccessToken(), refreshToken: c.exposeRefreshToken(), clientId: c.clientId, issuer: c.issuer, accessTokenExpiresAt: c.accessTokenExpiresAt });
const check = (condition, message) => { if (!condition) throw new Error(message); };
const gate = () => { let resolve; const promise = new Promise(done => { resolve = done; }); return { promise, resolve }; };
export const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
export const fixture = async (path, data) => {
  const response = await fetch(base + path, data === undefined ? {} : { method: 'POST', body: JSON.stringify(data) });
  check(response.ok, 'fixture HTTP failed'); return response.json();
};
const failure = async (code, operation) => {
  try { await operation(); throw new Error('expected failure'); }
  catch (error) {
    check(error.code === code, `incorrect error classification: ${error.code ?? 'missing'}; expected ${code}`);
    check(!String(error).includes('DO_NOT_LEAK'), 'host secret in diagnostic');
  }
};
const importSession = (store, saved = credentials()) => importZitadelSession(saved, store, environment);

export async function runCases(report) {
  let passed = 0;
  const test = async (name, fn) => { await fn(); passed++; report(`PASS ${name}`); };
  await test('import before I/O, durable ACK, concurrent waiters, restart', async () => {
    await fixture('/__reset', {}); await durable.clear();
    const entered = gate(), release = gate(); let saves = 0;
    const session = await importSession(async c => {
      saves++; check(!JSON.stringify(c).includes('access-1') && String(c).includes('redacted'), 'snapshot must redact');
      entered.resolve(); await release.promise; await durable.save(extract(c));
    });
    const zero = await fixture('/__stats'); check(zero.refresh === 0 && zero.exchange === 0, 'import performed auth I/O');
    const calls = [Auki.accessibleDomains(session), Auki.accessibleDomains(session)];
    await entered.promise;
    check((await fixture('/__stats')).exchange === 0 && await durable.load() === null, 'exchange before durable ACK');
    // A stale ACK must not release this generation.
    check(await Auki._ackZitadelSave(session, 'stale-request', true) === false, 'stale ACK accepted');
    release.resolve();
    const results = await Promise.all(calls);
    check(results.every(d => d.length === 1 && d[0].name === 'Readable Domain'), 'Domain result changed');
    check(saves === 1 && (await fixture('/__stats')).refresh === 1, 'single flight lost across Expo');
    const saved = await durable.load(); check(saved.refreshToken === 'refresh-1' && saved.accessTokenExpiresAt.endsWith('Z'), 'durable snapshot lost rotation or timestamp');
    await closeSession(session);
    const restarted = await importSession(async () => { throw new Error('unexpected refresh'); }, saved);
    check((await Auki.accessibleDomains(restarted)).length === 1, 'restart failed');
    check((await fixture('/__stats')).refresh === 1, 'restart rotated saved generation');
    await closeSession(restarted); await durable.clear();
  });
  await test('save rejection/retry retains same generation', async () => {
    await fixture('/__reset', {}); let fail = true; const generations = [];
    const session = await importSession(async c => {
      generations.push(c.exposeRefreshToken());
      if (fail) throw new Error('DO_NOT_LEAK_HOST_SECRET');
      await durable.save(extract(c));
    });
    await failure('persistence', () => Auki.accessibleDomains(session));
    check((await fixture('/__stats')).exchange === 0, 'rejected save allowed exchange');
    fail = false;
    check((await Auki.accessibleDomains(session)).length === 1, 'save retry failed');
    check(generations.join(',') === 'refresh-1,refresh-1' && (await fixture('/__stats')).refresh === 1, 'save retry replayed refresh');
    await closeSession(session); await durable.clear();
  });
  await test('missing Promise is failure, not an implicit ACK', async () => {
    await fixture('/__reset', {}); let correct = false;
    const session = await importSession(c => correct ? durable.save(extract(c)).then(() => {}) : undefined);
    await failure('persistence', () => Auki.accessibleDomains(session));
    correct = true; await Auki.accessibleDomains(session);
    check((await fixture('/__stats')).refresh === 1, 'missing-Promise recovery replayed refresh');
    await closeSession(session); await durable.clear();
  });
  await test('startup failure keeps session usable; unreadable Domain denied', async () => {
    await fixture('/__reset', { exchangeFailure: 503 }); let saves = 0;
    const session = await importSession(async c => { saves++; await durable.save(extract(c)); });
    await failure('transient', () => Auki.accessibleDomains(session));
    await fixture('/__configure', { exchangeFailure: null });
    await Auki.accessibleDomains(session);
    await failure('authorization_denied', () => Auki.startPeer(session, '00000000-0000-0000-0000-000000000099'));
    check(saves === 1, 'startup retry lost replacement');
    await closeSession(session); await durable.clear();
  });
  await test('logout awaits outstanding write before clearing storage', async () => {
    await fixture('/__reset', {});
    const entered = gate(), release = gate(); let closed = false;
    const session = await importSession(async c => { entered.resolve(); await release.promise; await durable.save(extract(c)); });
    const lookup = failure('closed', () => Auki.accessibleDomains(session));
    await entered.promise;
    const closing = closeSession(session).then(() => { closed = true; });
    await delay(150); check(!closed, 'close returned before outstanding write settled');
    release.resolve(); await Promise.all([lookup, closing]);
    await durable.clear(); await closeSession(session); await delay(50);
    check(await durable.load() === null && (await fixture('/__stats')).exchange === 0, 'late write or exchange after logout');
    await failure('closed', () => Auki.accessibleDomains(session));
  });
  await test('typed terminal failures latch and configuration stays redacted', async () => {
    for (const [tokenError, code] of [['invalid_grant', 'authentication_required'], ['invalid_client', 'configuration']]) {
      await fixture('/__reset', { tokenError });
      const session = await importSession(async () => { throw new Error('must not save'); });
      await failure(code, () => Auki.accessibleDomains(session)); await failure(code, () => Auki.accessibleDomains(session));
      check((await fixture('/__stats')).refresh === 1, 'terminal error retried automatically');
      await closeSession(session);
    }
    await failure('configuration', () => importSession(async () => {}, { ...credentials(), issuer: 'DO_NOT_LEAK_SECRET' }));
  });
  await test('real background/freeze and resume preserves pending save', async () => {
    await fixture('/__reset', {});
    const entered = gate(), release = gate(); let saves = 0;
    const session = await importSession(async c => { saves++; entered.resolve(); await release.promise; await durable.save(extract(c)); });
    const states = [];
    const subscription = AppState.addEventListener('change', value => states.push(value));
    const lookup = Auki.accessibleDomains(session).then(value => ({ value }), error => ({ error }));
    await entered.promise;
    await fixture('/__phase', { phase: 'suspend-ready' });
    // External harness freezes Chrome or backgrounds the actual simulator app.
    const started = Date.now();
    while ((await fixture('/__phase')).phase !== 'resumed') {
      if (Date.now() - started > 60000) throw new Error('external suspension driver missing');
      await delay(100);
    }
    if (Platform.OS === 'ios') {
      // simctl's launch response precedes UIKit's foreground notification. Wait
      // for the actual transition, not just the external driver's acknowledgement.
      const foregroundDeadline = Date.now() + 5000;
      while (AppState.currentState !== 'active' && Date.now() < foregroundDeadline) await delay(50);
      check(states.includes('background') && AppState.currentState === 'active',
        `simulator lifecycle incomplete: ${states.join(',')}; current ${AppState.currentState}`);
    }
    check((await fixture('/__stats')).exchange === 0, 'work crossed unacknowledged save while suspended');
    release.resolve();
    const result = await lookup;
    if (result.error) {
      check(result.error.code === 'transient', 'unexpected failure while suspended');
      // The bounded observing call can expire while suspended. Its native save
      // still belongs to this session; resume waiting without a new rotation.
      await Auki.accessibleDomains(session);
    }
    subscription.remove();
    check(saves === 1 && (await fixture('/__stats')).refresh === 1, 'resume duplicated refresh or save');
    await closeSession(session); await durable.clear();
  });
  report(`PASS ${passed} Expo host cases`);
  await fixture('/__phase', { phase: 'passed', count: passed });
}
