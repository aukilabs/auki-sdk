import { CHAT_PROTOCOL } from './chat.ts';
interface Candidate {
  readonly peerId: string; readonly routes: string[]; readonly servedProtocols: string[];
  readonly expiresAt: string; free(): void;
}
interface DiscoveryPeer {
  discoverProtocol(protocol: string): Promise<Candidate[]>; shutdown(): Promise<void>; free(): void;
}
export type ChatTarget = { peerId: string; route: string; expiresAt: number };
export function exactChatRoute(peer: string, route: string) {
  return !!peer && /^\/dns4\/[^/]+\/tcp\/\d+\/wss\/p2p\/[^/]+\/p2p-circuit\/p2p\/[^/]+$/.test(route) && route.endsWith('/p2p/' + peer);
}
function failure(error: unknown): 'denied' | 'offline' | 'error' {
  // The current discovery binding returns Error strings, not structured HTTP errors.
  const message = error instanceof Error ? error.message : String(error);
  if (/\b(401|403)\b|denied|forbidden|unauthorized/i.test(message)) return 'denied';
  if (/network|fetch|offline|timed? ?out|timeout|unavailable|transport failed|connect/i.test(message)) return 'offline';
  return 'error';
}
export class ChatDiscovery {
  state: 'stopped' | 'searching' | 'ready' | 'empty' | 'denied' | 'offline' | 'error' | 'closing' = 'stopped';
  candidates: ChatTarget[] = []; selected?: ChatTarget;
  private generation = 0; private pending?: Promise<void>; private closing?: Promise<void>;
  private cleanupFailed = false; private changed: () => void;
  constructor(changed: () => void) { this.changed = changed; }
  select(index: number) {
    const target = this.candidates[index];
    this.selected = this.state === 'ready' && target?.expiresAt > Date.now() ? target : undefined;
    this.changed(); return this.selected;
  }
  find(makePeer: () => Promise<DiscoveryPeer>, timeout = 30_000): Promise<void> {
    if (this.closing) return this.closing;
    if (this.cleanupFailed) return Promise.reject(new Error('Chat discovery cleanup failed.'));
    const generation = ++this.generation, previous = this.pending;
    this.candidates = []; this.selected = undefined; this.state = 'searching'; this.changed();
    const pending = (async () => {
      await previous;
      if (generation !== this.generation) return;
      let peer: DiscoveryPeer | undefined;
      const expire = () => {
        if (generation !== this.generation) return;
        ++this.generation; this.candidates = []; this.selected = undefined; this.state = 'offline'; this.changed();
      };
      let timer = setTimeout(expire, timeout);
      let targets: ChatTarget[] = [], state: ChatDiscovery['state'] = 'empty';
      try {
        peer = await makePeer();
        if (generation !== this.generation) return;
        clearTimeout(timer); timer = setTimeout(expire, Math.min(timeout, 15_000));
        const wrappers = await peer.discoverProtocol(CHAT_PROTOCOL);
        try {
          if (generation !== this.generation) return;
          for (const candidate of wrappers) {
            const peerId = candidate.peerId, expiresAt = Date.parse(candidate.expiresAt);
            if (!candidate.servedProtocols.includes(CHAT_PROTOCOL) || !(expiresAt > Date.now())) continue;
            for (const route of candidate.routes) if (exactChatRoute(peerId, route)) targets.push({ peerId, route, expiresAt });
          }
          state = targets.length ? 'ready' : 'empty';
        } finally {
          for (const candidate of wrappers) {
            try { candidate.free(); } catch { this.cleanupFailed = true; }
          }
        }
      } catch (error) { state = failure(error); targets = []; }
      finally {
        clearTimeout(timer);
        // SDK discovery is nonabortable: settle it before shutdown/free, including stale results.
        try { await peer?.shutdown(); } catch { this.cleanupFailed = true; }
        try { peer?.free(); } catch { this.cleanupFailed = true; }
        if (generation === this.generation) {
          this.candidates = this.cleanupFailed ? [] : targets;
          this.state = this.cleanupFailed ? 'error' : state; this.changed();
        }
        if (this.cleanupFailed) throw new Error('Chat discovery cleanup failed.');
      }
    })();
    this.pending = pending;
    return pending;
  }
  close(): Promise<void> {
    if (this.closing) return this.closing;
    ++this.generation; this.candidates = []; this.selected = undefined; this.state = 'closing'; this.changed();
    this.closing = (async () => {
      try { await this.pending; }
      finally {
        this.pending = undefined; this.state = this.cleanupFailed ? 'error' : 'stopped'; this.changed();
      }
      if (this.cleanupFailed) throw new Error('Chat discovery cleanup failed.');
    })().finally(() => { this.closing = undefined; });
    return this.closing;
  }
}
