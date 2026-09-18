import test from 'node:test';
import assert from 'node:assert/strict';
import { ChatDiscovery, exactChatRoute } from '../src/chat-discovery.ts';
import { CHAT_PROTOCOL } from '../src/chat.ts';
const route = '/dns4/host.test/tcp/443/wss/p2p/relay/p2p-circuit/p2p/remote';
const tick = () => new Promise(resolve => setTimeout(resolve, 0));
function fixture() {
  const events: string[] = [];
  const candidate = { peerId: 'remote', routes: [route], servedProtocols: [CHAT_PROTOCOL], expiresAt: new Date(Date.now() + 60000).toISOString(), free: () => { events.push('candidate free'); } };
  const peer = { discoverProtocol: async (protocol: string) => { assert.equal(protocol, CHAT_PROTOCOL); return [candidate]; }, shutdown: async () => { events.push('shutdown'); }, free: () => { events.push('peer free'); } };
  return { events, candidate, peer, discovery: new ChatDiscovery(() => {}) };
}
test('explicit exact Chat discovery copies candidates then frees and shuts down before selection', async () => {
  const f = fixture(); assert.deepEqual(f.events, []);
  await f.discovery.find(async () => f.peer);
  assert.equal(f.discovery.state, 'ready'); assert.equal(f.discovery.select(0)?.route, route);
  assert.deepEqual(f.events, ['candidate free', 'shutdown', 'peer free']);
  assert.equal(exactChatRoute('other', route), false);
  await f.discovery.close(); assert.equal(f.discovery.selected, undefined);
});
test('late discovery cannot revive Domain/logout/close and peer is not freed while SDK call is pending', async () => {
  const f = fixture(); let resolve!: (value: typeof f.candidate[]) => void;
  f.peer.discoverProtocol = () => new Promise(ok => { resolve = ok; });
  const finding = f.discovery.find(async () => f.peer); await tick();
  const closing = f.discovery.close(); assert.deepEqual(f.events, []);
  resolve([f.candidate]); await finding; await closing;
  assert.equal(f.discovery.state, 'stopped'); assert.deepEqual(f.discovery.candidates, []);
  assert.deepEqual(f.events, ['candidate free', 'shutdown', 'peer free']);
});
test('refresh immediately fences selection and drains previous lookup', async () => {
  const f = fixture(); await f.discovery.find(async () => f.peer); f.discovery.select(0);
  const refresh = f.discovery.find(async () => f.peer); assert.equal(f.discovery.selected, undefined);
  await refresh; assert.equal(f.discovery.state, 'ready'); await f.discovery.close();
});
test('Echo-only, expired and mismatched routes are excluded and freed', async () => {
  for (const change of [{ servedProtocols: ['/echo'] }, { expiresAt: '2000-01-01' }, { routes: [route + 'wrong'] }]) {
    const f = fixture(); Object.assign(f.candidate, change); await f.discovery.find(async () => f.peer);
    assert.equal(f.discovery.state, 'empty'); assert.equal(f.events[0], 'candidate free');
  }
});
test('all wrappers freed on getter failure and cleanup failure remains observable', async () => {
  const f = fixture(); const second = { ...f.candidate, free: () => { f.events.push('second free'); } };
  Object.defineProperty(f.candidate, 'peerId', { get() { throw new Error('malformed'); } });
  f.peer.discoverProtocol = async () => [f.candidate, second];
  await f.discovery.find(async () => f.peer); assert.equal(f.discovery.state, 'error');
  assert.deepEqual(f.events, ['candidate free', 'second free', 'shutdown', 'peer free']);
  f.peer.shutdown = async () => { throw new Error('cleanup'); };
  await assert.rejects(f.discovery.find(async () => f.peer)); await assert.rejects(f.discovery.close());
});
test('denied, offline, generic failure and empty remain distinct', async () => {
  for (const [message, state] of [['403 forbidden', 'denied'], ['network request failed', 'offline'], ['invalid response', 'error']]) {
    const f = fixture(); f.peer.discoverProtocol = async () => { throw new Error(message); };
    await f.discovery.find(async () => f.peer); assert.equal(f.discovery.state, state);
    assert.deepEqual(f.events, ['shutdown', 'peer free']);
  }
});
test('refresh during a pending lookup fences stale wrappers and serializes peer ownership', async () => {
  const f = fixture(); let resolve!: (value: typeof f.candidate[]) => void;
  f.peer.discoverProtocol = () => new Promise(ok => { resolve = ok; });
  const first = f.discovery.find(async () => f.peer); await tick();
  const next = fixture(); let started = false;
  const second = f.discovery.find(async () => { started = true; return next.peer; });
  await tick(); assert.equal(started, false);
  resolve([f.candidate]); await first; await second;
  assert.deepEqual(f.events, ['candidate free', 'shutdown', 'peer free']);
  assert.equal(f.discovery.state, 'ready'); assert.equal(f.discovery.candidates.length, 1);
  await f.discovery.close();
});
test('deadline fences a nonabortable late lookup and close still waits for its cleanup', async () => {
  const f = fixture(); let resolve!: (value: typeof f.candidate[]) => void;
  f.peer.discoverProtocol = () => new Promise(ok => { resolve = ok; });
  const finding = f.discovery.find(async () => f.peer, 5);
  await new Promise(ok => setTimeout(ok, 20));
  assert.equal(f.discovery.state, 'offline'); assert.deepEqual(f.events, []);
  const closing = f.discovery.close(); resolve([f.candidate]); await finding; await closing;
  assert.equal(f.discovery.state, 'stopped'); assert.deepEqual(f.discovery.candidates, []);
  assert.deepEqual(f.events, ['candidate free', 'shutdown', 'peer free']);
});
test('close during startup drains the late peer without invoking discovery', async () => {
  const f = fixture(); let resolve!: (value: typeof f.peer) => void;
  const finding = f.discovery.find(() => new Promise(ok => { resolve = ok; })); await tick();
  const closing = f.discovery.close(); assert.equal(closing, f.discovery.close()); resolve(f.peer);
  await finding; await closing;
  assert.deepEqual(f.events, ['shutdown', 'peer free']);
});
