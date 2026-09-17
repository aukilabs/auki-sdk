import { test } from 'node:test';
import assert from 'node:assert/strict';
import { Connection } from '../src/sdk.ts';
import type { AukiUserSession } from '../../portable-echo/web/pkg-web/auki_portable_echo_web.js';

function deferred<T = void>() {
  let resolve!: (value: T | PromiseLike<T>) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
const checkpoint = () => new Promise<void>(resolve => setImmediate(resolve));

for (const failure of ['callback-rejection', 'callback-throw', 'data-throw', 'data-rejection'] as const) {
  test(`selection drains earlier cleanup and old data before surfacing ${failure}, including concurrent logout`, async () => {
    const connection = new Connection(), events: string[] = [];
    const dataDrain = deferred(), earlierDrain = deferred(), callbackDrain = deferred();
    await connection.accept(Promise.resolve({
      data: (id: string) => { events.push(`create ${id}`); return { close: () => {
        events.push('data start');
        if (failure === 'data-throw') throw new Error('private data error');
        return dataDrain.promise.then(() => { events.push('data end'); if (failure === 'data-rejection') throw new Error('private data error'); });
      } }; },
      close: async () => { events.push('session'); },
    } as unknown as AukiUserSession));
    await connection.select('A');
    connection.beforeClose = () => earlierDrain.promise.then(() => { events.push('earlier end'); });
    const earlier = connection.select('B');
    connection.beforeClose = () => {
      if (failure === 'callback-throw') throw new Error('private callback error');
      if (failure === 'callback-rejection') return Promise.reject(new Error('private callback error'));
      return callbackDrain.promise.then(() => { events.push('callback end'); });
    };
    const selecting = connection.select('C');
    let selectionSettled = false;
    void selecting.then(() => { selectionSettled = true; }, () => { selectionSettled = true; });
    const selectionResults = Promise.allSettled([earlier, selecting]);
    await checkpoint();
    const failedEarly = selectionSettled;
    const closing = connection.close(), closingAgain = connection.close();
    const closeResults = Promise.allSettled([closing, closingAgain]);
    await checkpoint();
    const closedEarly = events.includes('session');
    // Release children separately: no individual drain may substitute for the others.
    if (failure.startsWith('data-')) dataDrain.resolve();
    else earlierDrain.resolve();
    callbackDrain.resolve();
    await checkpoint();
    const settledBeforeLastChild = selectionSettled || events.includes('session');
    earlierDrain.resolve(); dataDrain.resolve();
    const results = await selectionResults;
    const closed = await closeResults;
    assert.equal(failedEarly, false, 'selection must retain earlier cleanup even after another child fails');
    assert.equal(closedEarly, false, 'logout must retain every child');
    assert.equal(settledBeforeLastChild, false, 'the last outstanding child must drain too');
    assert.equal(results[1].status, 'rejected');
    assert.ok(closed.every(result => result.status === 'rejected'));
    assert.equal(events.at(-1), 'session');
    assert.equal(events.filter(event => event === 'session').length, 1);
    assert.ok(events.includes('data start'));
    assert.ok(!events.includes('create B') && !events.includes('create C'));
    if (failure !== 'data-throw') assert.ok(events.includes('data end'));
    connection.beforeClose = async () => {};
    await connection.accept(Promise.resolve({ data: () => ({ close: async () => {} }), close: async () => {} } as unknown as AukiUserSession));
    assert.ok(await connection.select('recovered'));
    await connection.close();
  });
}

test('synchronous logout callback failure still starts data cleanup and coalesces session-last teardown', async () => {
  const connection = new Connection(), drain = deferred(), events: string[] = [];
  await connection.accept(Promise.resolve({
    data: () => ({ close: () => { events.push('data start'); return drain.promise.then(() => { events.push('data end'); }); } }),
    close: async () => { events.push('session'); },
  } as unknown as AukiUserSession));
  await connection.select('A');
  connection.beforeClose = () => { throw new Error('private callback'); };
  const results = Promise.allSettled([connection.close(), connection.close()]);
  await checkpoint();
  const beforeDrain = [...events];
  drain.resolve();
  const outcomes = await results;
  assert.deepEqual(beforeDrain, ['data start']);
  assert.deepEqual(events, ['data start', 'data end', 'session']);
  assert.ok(outcomes.every(result => result.status === 'rejected' && result.reason.message === 'Cleanup failed.'));
});

test('new login waits for old teardown and concurrent logout fences it without losing either session', async () => {
  const connection = new Connection(), drain = deferred(), entered = deferred(), events: string[] = [];
  await connection.accept(Promise.resolve({ close: async () => { entered.resolve(); await drain.promise; events.push('old session'); } } as unknown as AukiUserSession));
  const closing = connection.close(); await entered.promise;
  const accepting = connection.accept(Promise.resolve({ close: async () => { events.push('new session'); } } as unknown as AukiUserSession));
  await checkpoint();
  const publishedEarly = connection.session !== undefined;
  const again = connection.close();
  drain.resolve();
  await Promise.all([closing, again]);
  assert.equal(await accepting, false);
  assert.equal(publishedEarly, false);
  assert.equal(connection.session, undefined);
  assert.deepEqual(events, ['old session', 'new session']);
});


test('selection during logout creates no new cleanup and a waiting new login recovers after old failure', async () => {
  const connection = new Connection(), drain = deferred(), entered = deferred();
  let hooks = 0, oldClosed = false, newClosed = false;
  await connection.accept(Promise.resolve({ close: async () => { entered.resolve(); await drain.promise; oldClosed = true; throw new Error('private'); } } as unknown as AukiUserSession));
  connection.beforeClose = async () => { hooks++; };
  const closing = connection.close();
  const outcome = Promise.allSettled([closing]);
  await entered.promise;
  const stale = connection.select('stale');
  const accepting = connection.accept(Promise.resolve({
    data: () => ({ close: async () => {} }), close: async () => { newClosed = true; },
  } as unknown as AukiUserSession));
  await checkpoint();
  assert.equal(connection.session, undefined);
  assert.equal(hooks, 1);
  drain.resolve();
  await outcome;
  assert.equal(await stale, undefined);
  assert.equal(await accepting, true);
  assert.ok(oldClosed);
  assert.ok(await connection.select('new'));
  await connection.close();
  assert.ok(newClosed);
});
