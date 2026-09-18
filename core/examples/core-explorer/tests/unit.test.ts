import { test } from 'node:test';
import assert from 'node:assert/strict';
import { endpoint, uuid, inspect, safeError, ReadLane, previewBytes, redact } from '../src/safety.ts';
test('endpoint rejects credentials, insecure remote hosts and URL payloads', () => {
  for (const value of ['http://example.com', 'https://user:secret@example.com', 'https://example.com/?token=secret', 'ftp://127.0.0.1', 'https://example.com/#secret']) assert.throws(() => endpoint(value));
  assert.equal(endpoint('http://127.0.0.1:18114/'), 'http://127.0.0.1:18114');
  assert.equal(endpoint('https://example.com/'), 'https://example.com');
});
test('UUID selection validates complete IDs', () => {
  assert.throws(() => uuid('not-an-id'));
  assert.equal(uuid('AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA'), 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa');
});
test('inspector recursively redacts credential fields and bearer/JWT patterns', () => {
  const output = inspect({ nested: [{ accessToken: 'secret', private_key: 'private' }], text: 'Bearer opaque-secret e30.e30.signature' });
  for (const value of ['opaque-secret', 'signature', '"private"', '"secret"']) assert.ok(!output.includes(value));
  assert.ok(output.includes('[redacted]'));
});
test('errors never include backend message bodies', () => {
  assert.equal(safeError({ status: 403, message: 'secret' }), 'Denied — this read is not authorized.');
  assert.ok(!safeError(new Error('secret')).includes('secret'));
});
test('opaque bearer credentials after labels are redacted in text and nested JSON notes', () => {
  for (const label of ['Authorization:', 'authorization=', 'AUTHORIZATION \t=\t', 'Authorization \t: \n', 'access_token=']) {
    for (const bearer of ['Bearer ', 'bEaReR\t', 'BEARER \n\t']) {
      const note = `Useful text ${label}${bearer}SYNTHETIC_OPAQUE_SECRET done`;
      for (const value of [note, JSON.stringify({ nested: [{ note }] })]) {
        const output = previewBytes(new TextEncoder().encode(value));
        assert.ok(!output.includes('SYNTHETIC_OPAQUE_SECRET'));
        assert.ok(output.includes('[redacted]'));
        assert.ok(output.includes('Useful text'));
        assert.ok(output.includes('done'));
      }
    }
  }
});
test('late requests cannot overwrite newer results and cancellation reaches SDK', async () => {
  const lane = new ReadLane(); let release!: (value: string) => void; let signal!: AbortSignal; const results: string[] = [];
  const old = lane.run(s => { signal = s; return new Promise<string>(resolve => { release = resolve; }); }, value => results.push(value), () => assert.fail());
  await lane.run(async () => 'new', value => results.push(value), () => assert.fail());
  release('old'); await old;
  assert.ok(signal.aborted); assert.deepEqual(results, ['new']);
});
test('timeout aborts a pending read and reports a sanitized timeout', async () => {
  const lane = new ReadLane(); let error: unknown;
  await lane.run(signal => new Promise((_, reject) => signal.addEventListener('abort', () => reject(new Error('secret')))), () => assert.fail(), value => { error = value; }, 5);
  assert.equal(safeError(error), 'Timed out — retry this read.');
});
test('truncated JSON preview still redacts quoted credential values', () => {
  assert.ok(!inspect('{"access_token": "secret value", "tail": ').includes('secret value'));
  assert.ok(!inspect('{"refresh_token": "unfinished secret').includes('unfinished secret'));
});

// Unit doubles assert ordering only; browser integration uses generated WASM.
import { Connection } from '../src/sdk.ts';
import type { AukiUserSession } from '../../portable-echo/web/pkg-web/auki_portable_echo_web.js';
test('logout awaits client closure before session closure', async () => {
  const events: string[] = []; let release!: () => void;
  const data = { close: () => new Promise<void>(resolve => { events.push('client closing'); release = () => { events.push('client closed'); resolve(); }; }) };
  const session = { data: () => data, close: async () => { events.push('session closed'); } } as unknown as AukiUserSession;
  const connection = new Connection(); await connection.accept(Promise.resolve(session)); await connection.select('domain');
  const closing = connection.close(); assert.deepEqual(events, ['client closing']);
  release(); await closing; assert.deepEqual(events, ['client closing', 'client closed', 'session closed']);
});
test('cancelled login closes a late session without accepting it', async () => {
  let release!: (value: AukiUserSession) => void; let closed = false;
  const connection = new Connection();
  const accepting = connection.accept(new Promise(resolve => { release = resolve; }));
  await connection.close();
  release({ close: async () => { closed = true; } } as unknown as AukiUserSession);
  assert.equal(await accepting, false); assert.ok(closed); assert.equal(connection.session, undefined);
});

test('full JSON is recursively redacted before the 64 KiB display cap', () => {
  const value = JSON.stringify({ nested: [{ private_key: 'SYNTHETIC_PRIVATE', api_key: 'SYNTHETIC_API', access_token: 'SYNTHETIC_START"escaped\\SYNTHETIC_END' }], tail: 'é'.repeat(66000) });
  const output = previewBytes(new TextEncoder().encode(value));
  assert.ok(output.startsWith('Truncated to 64 KiB.'));
  assert.ok(new TextEncoder().encode(output).length <= 65536);
  assert.ok(!output.includes('SYNTHETIC_'));
  assert.ok(!output.includes('�'));
});
test('malformed and truncated JSON previews are withheld', () => {
  for (const value of ['{"private_key":"SYNTHETIC_PRIVATE",', '[{"api_key":"SYNTHETIC_API"}', '{"access_token":"SYNTHETIC_UNFINISHED', JSON.stringify({ api_key: 'SYNTHETIC_API', tail: 'x'.repeat(66000) }).slice(0, 65536)]) {
    assert.equal(previewBytes(new TextEncoder().encode(value)), 'Preview withheld — malformed or incomplete JSON.');
  }
});
test('text credentials use consistent labels and escape-aware quoted values', () => {
  for (const key of ['private_key', 'api_key', 'access_token', 'refreshToken', 'password', 'secret', 'credential', 'authorization', 'cookie']) {
    for (const value of ['"SYNTHETIC_START\\"SYNTHETIC_END"', "'SYNTHETIC_START\\'SYNTHETIC_END'", '"SYNTHETIC_START\\\\SYNTHETIC_END"', '"SYNTHETIC_UNFINISHED', '"SYNTHETIC_START SYNTHETIC_END\\', '"Bearer SYNTHETIC_START\\"SYNTHETIC_END"', "'Bearer SYNTHETIC_START\\'SYNTHETIC_END'"]) {
      assert.ok(!String(redact(`Useful text ${key}=${value}`)).includes('SYNTHETIC_'));
    }
  }
  assert.equal(previewBytes(new TextEncoder().encode('Useful text api_key=SYNTHETIC_API; done')), 'Useful text api_key=[redacted]; done');
});

test('large plain ASCII preview stays bounded and useful', () => {
  const output = previewBytes(new TextEncoder().encode('x'.repeat(100000)));
  assert.ok(output.startsWith('Truncated to 64 KiB.\nxxxx'));
  assert.equal(new TextEncoder().encode(output).length, 65536);
});

import { Networking, diagnosticBytes } from '../src/networking.ts';
test('diagnostic validates UTF-8 byte bounds', () => {
  assert.throws(() => diagnosticBytes(''));
  assert.throws(() => diagnosticBytes('é'.repeat(513)));
  assert.equal(diagnosticBytes('é'.repeat(512)).length, 1024);
});
test('stop awaits late startup cleanup and prevents acceptance', async () => {
  const events: string[] = []; let release!: (value: any) => void;
  const net = new Networking(() => {});
  const starting = net.start(() => new Promise(resolve => { release = resolve; }));
  const stopping = net.stop().then(() => events.push('stopped'));
  assert.equal(net.state, 'stopping');
  release({ shutdown: async () => { events.push('peer closed'); }, free: () => {} });
  await Promise.all([starting, stopping]);
  assert.deepEqual(events, ['peer closed', 'stopped']);
  assert.equal(net.state, 'stopped'); assert.equal(net.peer, undefined);
});

test('logout waits for late peer cleanup before closing the shared session', async () => {
  const events: string[] = []; let release!: (value: any) => void;
  const net = new Networking(() => {}), connection = new Connection();
  connection.beforeClose = () => net.stop();
  await connection.accept(Promise.resolve({ close: async () => { events.push('session'); } } as unknown as AukiUserSession));
  const starting = net.start(() => new Promise(resolve => { release = resolve; }));
  const closing = connection.close();
  assert.deepEqual(events, []);
  release({ shutdown: async () => { events.push('peer'); }, free: () => {} });
  await Promise.all([starting, closing]);
  assert.deepEqual(events, ['peer', 'session']);
});
test('late diagnostic is discarded and disposed before freeing client or session', async () => {
  const events: string[] = []; let release!: (value: string) => void;
  const net = new Networking(() => {});
  await net.start(async () => ({ shutdown: async () => { events.push('shutdown'); }, free: () => { events.push('peer freed'); } }), () => ({ free: () => { events.push('client freed'); } }));
  const sending = net.run(() => new Promise<string>(resolve => { release = resolve; }), () => assert.fail('stale response rendered'), () => assert.fail(), () => { events.push('receipt freed'); });
  const stopping = net.stop();
  await Promise.resolve();
  assert.deepEqual(events, ['shutdown']);
  release('Bearer SYNTHETIC_SECRET');
  await Promise.all([sending, stopping]);
  assert.deepEqual(events, ['shutdown', 'receipt freed', 'client freed', 'peer freed']);
});
test('terminal peer failure closes resources and permits an explicit restart', async () => {
  let terminal!: () => void;
  const net = new Networking(() => {});
  await net.start(async () => ({ shutdown: async () => {}, free: () => {}, waitStopped: () => new Promise<void>(resolve => { terminal = resolve; }) }));
  terminal(); await Promise.resolve(); await net.stop();
  assert.equal(net.state, 'failed');
  await net.start(async () => ({ shutdown: async () => {}, free: () => {} }));
  assert.equal(net.state, 'ready'); await net.stop(); assert.equal(net.state, 'stopped');
});
test('startup deadline invalidates late acceptance while retaining cleanup ownership', async () => {
  let release!: (value: any) => void;
  let timedOut!: () => void;
  const stopped = new Promise<void>(resolve => { timedOut = resolve; });
  const net = new Networking(() => { if (net.state === 'stopping') timedOut(); });
  const starting = net.start(() => new Promise(resolve => { release = resolve; }), undefined, 5);
  await stopped;
  let closed = false;
  release({ shutdown: async () => { closed = true; }, free: () => {} });
  await starting; await net.stop();
  assert.ok(closed); assert.equal(net.state, 'failed');
});
test('late startup cleanup failure is reported as failed', async () => {
  let release!: (value: any) => void;
  const net = new Networking(() => {});
  const starting = net.start(() => new Promise(resolve => { release = resolve; }));
  const stopping = net.stop();
  release({ shutdown: async () => { throw new Error('synthetic secret'); }, free: () => {} });
  await Promise.all([starting, stopping]); assert.equal(net.state, 'failed');
});
test('logout still closes the session when data cleanup fails', async () => {
  let closed = false;
  const connection = new Connection();
  await connection.accept(Promise.resolve({ close: async () => { closed = true; }, data: () => ({ close: async () => { throw new Error('synthetic failure'); } }) } as unknown as AukiUserSession));
  await connection.select('domain');
  await assert.rejects(connection.close()); assert.ok(closed);
});
test('Domain switch waits for previous peer shutdown before creating data client', async () => {
  let release!: () => void;
  const events: string[] = [];
  const connection = new Connection(), net = new Networking(() => {});
  connection.beforeClose = () => net.stop();
  await connection.accept(Promise.resolve({ data: (id: string) => { events.push(id); return { close: async () => {} }; }, close: async () => {} } as unknown as AukiUserSession));
  await connection.select('domain A');
  await net.start(async () => ({ shutdown: () => new Promise<void>(resolve => { release = resolve; }), free: () => {} }));
  const selecting = connection.select('domain B');
  await Promise.resolve();
  assert.deepEqual(events, ['domain A']); assert.equal(net.state, 'stopping');
  release(); await selecting;
  assert.deepEqual(events, ['domain A', 'domain B']); assert.equal(net.state, 'stopped');
  await connection.close();
});

for (const failure of ['none', 'data', 'peer', 'session'] as const) {
  test(`concurrent logout awaits session teardown and permits relogin after ${failure} failure`, async () => {
    let release!: () => void, entered!: () => void;
    const sessionClosing = new Promise<void>(resolve => { entered = resolve; });
    const delayed = new Promise<void>(resolve => { release = resolve; });
    let closes = 0, hooks = 0;
    const connection = new Connection();
    connection.beforeClose = async () => { hooks++; };
    await connection.accept(Promise.resolve({
      data: () => ({ close: async () => { if (failure === 'data') throw new Error('private failure'); } }),
      close: async () => { closes++; entered(); await delayed; if (failure === 'session') throw new Error('private failure'); },
    } as unknown as AukiUserSession));
    await connection.select('first'); hooks = 0;
    connection.beforeClose = async () => { hooks++; if (failure === 'peer') throw new Error('private failure'); };
    const first = connection.close();
    await sessionClosing;
    let secondSettled = false;
    const second = connection.close();
    const outcomes = Promise.allSettled([first, second]);
    void second.then(() => { secondSettled = true; }, () => { secondSettled = true; });
    await new Promise(resolve => setImmediate(resolve));
    const settledEarly = secondSettled;
    release();
    const results = await outcomes;
    assert.equal(settledEarly, false, 'concurrent logout must await session.close');
    assert.equal(closes, 1); assert.equal(hooks, 1);
    for (const result of results) {
      assert.equal(result.status, failure === 'none' ? 'fulfilled' : 'rejected');
      if (result.status === 'rejected') assert.equal(result.reason.message, 'Cleanup failed.');
    }
    let recovered = false;
    connection.beforeClose = async () => {};
    await connection.accept(Promise.resolve({ data: () => ({ close: async () => {} }), close: async () => { recovered = true; } } as unknown as AukiUserSession));
    assert.ok(await connection.select('second'), 'cleanup failure must not poison next selection');
    await connection.close(); assert.ok(recovered);
  });
}

test('coalesced close still cancels a login begun during delayed session cleanup', async () => {
  const connection = new Connection();
  let release!: () => void, entered!: () => void, login!: (session: AukiUserSession) => void;
  const started = new Promise<void>(resolve => { entered = resolve; });
  const delayed = new Promise<void>(resolve => { release = resolve; });
  await connection.accept(Promise.resolve({ close: async () => { entered(); await delayed; } } as unknown as AukiUserSession));
  const first = connection.close(); await started;
  const accepting = connection.accept(new Promise(resolve => { login = resolve; }));
  const second = connection.close();
  let lateClosed = false;
  login({ close: async () => { lateClosed = true; } } as unknown as AukiUserSession);
  assert.equal(await accepting, false); assert.ok(lateClosed); assert.equal(connection.session, undefined);
  release(); await Promise.all([first, second]);
});

for (const failing of ['shutdown', 'client-free', 'peer-free']) test(`strict networking cleanup reports ${failing} failure and still closes shared session`, async () => {
  const events: string[] = [];
  const net = new Networking(() => {});
  await net.start(async () => ({ shutdown: async () => { events.push('shutdown'); if (failing === 'shutdown') throw new Error('private'); }, free: () => { events.push('peer-free'); if (failing === 'peer-free') throw new Error('private'); } }),
    () => ({ free: () => { events.push('client-free'); if (failing === 'client-free') throw new Error('private'); } }));
  const connection = new Connection();
  await connection.accept(Promise.resolve({ close: async () => { events.push('session-close'); } } as unknown as AukiUserSession));
  connection.beforeClose = () => net.close();
  await assert.rejects(connection.close(), /Cleanup failed/);
  assert.deepEqual(events, ['shutdown', 'client-free', 'peer-free', 'session-close']);
  await net.stop(); // UI/timer callers retain non-rejecting stop semantics.
  assert.equal(net.peer, undefined); assert.equal(net.client, undefined);
});
