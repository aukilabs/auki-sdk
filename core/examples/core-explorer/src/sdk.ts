import init, { AukiUserSession, type AukiDomainData } from '../../portable-echo/web/pkg-web/auki_portable_echo_web.js';
export type { DataMetadata, DataQuery, DomainSummary } from '../../portable-echo/web/pkg-web/auki_portable_echo_web.js';
let initialized: Promise<unknown> | undefined;
export async function login(urls: string[], email: string, password: string) {
  await (initialized ??= init());
  let id = localStorage.getItem('core-explorer.client-id');
  if (!id) { id = crypto.randomUUID(); localStorage.setItem('core-explorer.client-id', id); }
  return AukiUserSession.loginWithEnvironment(urls[0], urls[1], urls[2], email, password, id);
}
export { AukiDiscoveryMode, AukiPeerReachabilityMode, AukiEchoClient, type AukiPeer } from '../../portable-echo/web/pkg-web/auki_portable_echo_web.js';
// Start every child even if one throws synchronously; report failure only once
// all children have drained. Callers retain this aggregate until it settles.
export async function drainCleanup(...children: (() => unknown)[]): Promise<void> {
  const results = await Promise.allSettled(children.map(async child => child()));
  if (results.some(result => result.status === 'rejected')) throw new Error('Cleanup failed.');
}
export class Connection {
  beforeClose: () => Promise<void> = async () => {};
  session?: AukiUserSession;
  data?: AukiDomainData;
  private generation = 0;
  private selection = 0;
  private closing: Promise<void> = Promise.resolve();
  private teardown?: Promise<void>;
  async accept(pending: Promise<AukiUserSession>) {
    const generation = ++this.generation;
    ++this.selection;
    const session = await pending;
    // Do not publish a new session into an older, coalesced logout.
    if (generation === this.generation) await this.teardown?.catch(() => {});
    if (generation !== this.generation) { await session.close(); return false; }
    this.session = session;
    return true;
  }
  async select(id: string) {
    const selection = ++this.selection;
    const session = this.session;
    if (!session) { await this.closing; return; }
    const previous = this.data;
    this.data = undefined;
    const earlier = this.closing;
    this.closing = drainCleanup(() => earlier, () => this.beforeClose(), () => previous?.close());
    await this.closing;
    if (selection === this.selection && session && session === this.session) {
      this.data = session.data(id);
      return this.data;
    }
  }
  async close() {
    ++this.generation;
    ++this.selection;
    if (this.teardown) return this.teardown;
    const session = this.session;
    const data = this.data;
    this.session = undefined;
    this.data = undefined;
    const earlier = this.closing;
    const results = Promise.allSettled([drainCleanup(() => earlier, () => this.beforeClose(), () => data?.close())]);
    const teardown = (async () => {
      const cleanup = await results;
      const sessionResult = await Promise.allSettled([Promise.resolve().then(() => session?.close())]);
      if ([...cleanup, ...sessionResult].some(result => result.status === 'rejected')) throw new Error('Cleanup failed.');
    })();
    this.teardown = teardown;
    // Keep selection recovery nonsticky while all concurrent callers share errors.
    this.closing = teardown.then(() => undefined, () => undefined);
    try { await teardown; } finally { this.teardown = undefined; }
  }
}
