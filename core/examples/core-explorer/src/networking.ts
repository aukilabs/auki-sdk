export type NetworkState = 'stopped' | 'starting' | 'ready' | 'stopping' | 'failed';
interface ManagedPeer { shutdown(): Promise<void>; free(): void; waitStopped?(): Promise<void> }
interface Client { free(): void }
export function diagnosticBytes(value: string): Uint8Array {
  const bytes = new TextEncoder().encode(value);
  if (!bytes.length || bytes.length > 1024) throw new Error('Diagnostic must contain 1–1024 UTF-8 bytes.');
  return bytes;
}
// Nonabortable SDK calls stay owned until they settle. Generation fences prevent
// their results from reviving a stopped peer or crossing a Domain/session boundary.
export class Networking<P extends ManagedPeer = ManagedPeer, C extends Client = Client> {
  state: NetworkState = 'stopped';
  peer?: P;
  client?: C;
  private generation = 0;
  private cleanupFailed = false;
  private starting?: Promise<void>;
  private stopping?: Promise<void>;
  private operations = new Set<Promise<unknown>>();
  private changed: () => void;
  constructor(changed: () => void) { this.changed = changed; }
  start(factory: () => Promise<P>, mount?: (peer: P) => C, timeout = 30_000): Promise<void> {
    if (this.starting || this.stopping || this.peer) return Promise.resolve();
    this.cleanupFailed = false;
    const generation = ++this.generation;
    this.state = 'starting'; this.changed();
    const timer = setTimeout(() => { if (generation === this.generation) void this.stop(true); }, timeout);
    this.starting = (async () => {
      try {
        const peer = await factory();
        if (generation !== this.generation) { try { await peer.shutdown(); } catch { this.cleanupFailed = true; } finally { peer.free(); } return; }
        this.peer = peer;
        this.client = mount?.(peer);
        this.state = 'ready'; this.changed();
        void peer.waitStopped?.().then(() => {
          if (generation === this.generation) void this.stop(true);
        }, () => { if (generation === this.generation) void this.stop(true); });
      } catch {
        if (generation === this.generation) {
          // Also clean up a peer if adapter construction failed.
          if (this.peer) { try { await this.peer.shutdown(); } catch {} finally { this.peer.free(); this.peer = undefined; } }
          this.state = 'failed'; this.changed();
        }
      } finally { clearTimeout(timer); }
    })().finally(() => { this.starting = undefined; });
    return this.starting;
  }
  async run<T>(operation: (peer: P, client: C) => Promise<T>, success: (value: T) => void, failure: () => void, dispose: (value: T) => void = () => {}, timeout = 15_000) {
    if (this.state !== 'ready' || !this.peer || !this.client) return;
    const generation = this.generation;
    const timer = setTimeout(() => { if (generation === this.generation) { failure(); void this.stop(true); } }, timeout);
    const pending = (async () => {
      try {
        const value = await operation(this.peer!, this.client!);
        try { if (generation === this.generation) success(value); } finally { dispose(value); }
      } catch { if (generation === this.generation) failure(); }
      finally { clearTimeout(timer); }
    })();
    this.operations.add(pending);
    try { await pending; } finally { this.operations.delete(pending); }
  }
  stop(failed = false): Promise<void> {
    if (this.stopping) return this.stopping;
    ++this.generation;
    this.state = 'stopping'; this.changed();
    this.stopping = (async () => {
      await this.starting;
      const peer = this.peer, client = this.client;
      this.peer = undefined; this.client = undefined;
      try { await peer?.shutdown(); } catch { failed = true; }
      await Promise.allSettled([...this.operations]);
      client?.free(); peer?.free();
      this.state = failed || this.cleanupFailed ? 'failed' : 'stopped';
      this.stopping = undefined; this.changed();
    })();
    return this.stopping;
  }
}
