export const CHAT_PROTOCOL = '/example/core-explorer-chat/1.0.0';
export interface ChatConnection {
  sessionId: string; nextEvent(): Promise<string>; send(id: string, text: string): Promise<void>;
  close(): Promise<void>; free(): void;
}
export interface ChatPeer { readonly peerId: string; shutdown(): Promise<void>; free(): void; waitStopped(): Promise<void> }
export type ChatMessage = { id: string; text: string; direction: 'in' | 'out'; status: 'sending' | 'sent' | 'received' | 'unconfirmed' };
const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
export function chatText(text: string) {
  const size = new TextEncoder().encode(text).length;
  if (!size || size > 2048) throw new Error('Enter 1–2048 UTF-8 bytes.');
  return text;
}
export class Chat {
  state: 'stopped' | 'connecting' | 'pending' | 'paired' | 'closing' | 'failed' = 'stopped';
  sessionId = ''; peerId = ''; messages: ChatMessage[] = [];
  private generation = 0; private peer?: ChatPeer; private channel?: ChatConnection;
  private opening?: Promise<void>; private closing?: Promise<void>; private reading?: Promise<void>;
  private sends = new Set<Promise<void>>(); private seen = new Set<string>();
  private changed: () => void;
  constructor(changed: () => void) { this.changed = changed; }
  connect(makePeer: () => Promise<ChatPeer>, open: (peer: ChatPeer) => Promise<ChatConnection>) {
    if (this.opening || this.closing || this.peer) return Promise.resolve();
    const generation = ++this.generation;
    this.messages = []; this.seen.clear(); this.sessionId = ''; this.peerId = ''; this.state = 'connecting'; this.changed();
    this.opening = (async () => {
      const peer = await makePeer();
      this.peer = peer;
      if (generation !== this.generation) return;
      void peer.waitStopped().then(() => { if (generation === this.generation) void this.close(true).catch(() => {}); }, () => { if (generation === this.generation) void this.close(true).catch(() => {}); });
      const channel = await open(peer); this.channel = channel;
      if (generation !== this.generation) return;
      if (!uuid.test(channel.sessionId)) throw new Error('Invalid session');
      this.sessionId = channel.sessionId; this.peerId = peer.peerId; this.state = 'pending'; this.changed();
      this.reading = this.read(channel, generation);
    })().catch(() => { if (generation === this.generation) { this.state = 'failed'; this.changed(); queueMicrotask(() => { void this.close(true).catch(() => {}); }); } })
      .finally(() => { this.opening = undefined; });
    return this.opening;
  }
  private async read(channel: ChatConnection, generation: number) {
    try {
      while (generation === this.generation) {
        const raw = await channel.nextEvent();
        if (generation !== this.generation) return;
        if (raw.length > 16384) throw new Error('Oversize event');
        const event = JSON.parse(raw);
        if (event.type === 'paired' && this.state === 'pending') this.state = 'paired';
        else if (event.type === 'message' && this.state === 'paired' && uuid.test(event.id) && typeof event.text === 'string' && !this.seen.has(event.id)) {
          chatText(event.text);
          if (this.seen.size >= 128) throw new Error('Conversation limit');
          this.seen.add(event.id); this.messages.push({ id: event.id, text: event.text, direction: 'in', status: 'received' });
        } else if (event.type === 'ack' && this.state === 'paired') {
          const message = this.messages.find(m => m.id === event.id && m.direction === 'out' && ['sending', 'sent'].includes(m.status));
          if (!message) throw new Error('Unknown receipt');
          message.status = 'received';
        } else throw new Error('Invalid event');
        this.changed();
      }
    } catch { if (generation === this.generation) queueMicrotask(() => { void this.close(true).catch(() => {}); }); }
  }
  async send(text: string) {
    chatText(text);
    if (this.state !== 'paired' || !this.channel || this.seen.size >= 128) throw new Error('Chat unavailable or full');
    const channel = this.channel, generation = this.generation, id = crypto.randomUUID();
    const message: ChatMessage = { id, text, direction: 'out', status: 'sending' };
    this.seen.add(id); this.messages.push(message); this.changed();
    const pending = channel.send(id, text).then(() => {
      if (generation === this.generation && message.status === 'sending') message.status = 'sent';
    }, () => { if (generation === this.generation && message.status !== 'received') message.status = 'unconfirmed'; });
    this.sends.add(pending);
    try { await pending; } finally { this.sends.delete(pending); this.changed(); }
  }
  close(failed = false): Promise<void> {
    if (this.closing) return this.closing;
    ++this.generation; this.state = 'closing'; this.changed();
    this.closing = (async () => {
      await this.opening;
      let error = false;
      try { await this.channel?.close(); } catch { error = true; }
      await Promise.allSettled([this.reading, ...this.sends]);
      try { this.channel?.free(); } catch { error = true; }
      try { await this.peer?.shutdown(); } catch { error = true; }
      try { this.peer?.free(); } catch { error = true; }
      this.channel = undefined; this.peer = undefined; this.sessionId = ''; this.peerId = ''; this.messages = []; this.seen.clear();
      this.state = failed || error ? 'failed' : 'stopped'; this.changed();
      if (error) throw new Error('Chat cleanup failed');
    })().finally(() => { this.closing = undefined; });
    return this.closing;
  }
}
