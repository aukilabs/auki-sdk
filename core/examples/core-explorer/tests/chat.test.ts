import test from 'node:test';
import assert from 'node:assert/strict';
import { Chat, chatText, type ChatConnection } from '../src/chat.ts';
const sid = '11111111-1111-4111-8111-111111111111';
const tick = () => new Promise(resolve => setTimeout(resolve, 0));
function fixture() {
  const events: string[] = []; let receive: ((value: string) => void) | undefined, reject: ((error: Error) => void) | undefined;
  const channel: ChatConnection = { sessionId: sid, nextEvent: () => new Promise((ok, no) => { receive = ok; reject = no; }),
    send: async () => {}, close: async () => { events.push('chat close'); reject?.(new Error('closed')); }, free: () => { events.push('chat free'); } };
  const peer = { peerId: 'local-chat-peer', shutdown: async () => { events.push('peer shutdown'); }, free: () => { events.push('peer free'); }, waitStopped: () => new Promise<void>(() => {}) };
  const chat = new Chat(() => {});
  return { chat, channel, peer, events, emit: (event: unknown) => receive!(JSON.stringify(event)) };
}
test('UTF-8 size limit, pending denial, custom reply and truthful receipt', async () => {
  assert.throws(() => chatText('é'.repeat(1025))); assert.throws(() => chatText(''));
  const f = fixture(); await f.chat.connect(async () => f.peer, async () => f.channel);
  assert.equal(f.chat.state, 'pending'); assert.equal(f.chat.peerId, 'local-chat-peer'); await assert.rejects(f.chat.send('unapproved'));
  f.emit({ type: 'paired' }); await tick(); await f.chat.send('hello');
  assert.equal(f.chat.messages[0].status, 'sent');
  f.emit({ type: 'ack', id: f.chat.messages[0].id }); await tick();
  assert.equal(f.chat.messages[0].status, 'received');
  f.emit({ type: 'message', id: '22222222-2222-4222-8222-222222222222', text: 'a custom response' }); await tick();
  assert.equal(f.chat.messages[1].text, 'a custom response'); await f.chat.close();
  assert.deepEqual(f.events, ['chat close', 'chat free', 'peer shutdown', 'peer free']);
});
test('late open cannot revive after Domain/logout cleanup and close coalesces', async () => {
  const f = fixture(); let open!: (c: ChatConnection) => void;
  const starting = f.chat.connect(async () => f.peer, () => new Promise(resolve => { open = resolve; })); await tick();
  const close = f.chat.close(); assert.equal(close, f.chat.close()); open(f.channel); await starting; await close;
  assert.equal(f.chat.state, 'stopped'); assert.equal(f.chat.sessionId, ''); assert.equal(f.chat.peerId, '');
  assert.deepEqual(f.events, ['chat close', 'chat free', 'peer shutdown', 'peer free']);
});
test('preapproval payload and unknown acknowledgements close the connection', async () => {
  for (const event of [{ type: 'message', id: sid, text: 'bad' }, { type: 'ack', id: sid }]) {
    const f = fixture(); await f.chat.connect(async () => f.peer, async () => f.channel); f.emit(event); await tick(); await tick();
    assert.equal(f.chat.state, 'failed'); assert.deepEqual(f.chat.messages, []);
  }
});
test('close interrupts receive and discards late send completion', async () => {
  const f = fixture(); let finish!: () => void;
  f.channel.send = () => new Promise(resolve => { finish = resolve; });
  await f.chat.connect(async () => f.peer, async () => f.channel); f.emit({ type: 'paired' }); await tick();
  const sending = f.chat.send('hello'), closing = f.chat.close(); finish(); await sending; await closing;
  assert.deepEqual(f.chat.messages, []); assert.equal(f.chat.state, 'stopped');
});
