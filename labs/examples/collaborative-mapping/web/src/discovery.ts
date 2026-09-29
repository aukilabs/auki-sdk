/** Bounded per-peer subscriptions; Domain discovery stays active for the session lifetime. */
export interface Candidate {
  peerId: string;
  routes: string[];
  expiresAt: string;
}
export interface DiscoveryPorts {
  localPeer: string;
  discover(): Promise<Candidate[]>;
  inspect(peer: string, route: string): Promise<string | undefined>;
  follow(selection: string, peer: string): Promise<void>;
  unfollow(peer: string): Promise<void>;
  status(message: string): void;
}
export class DomainDiscovery {
  private readonly controller = new AbortController();
  private active = new Map<
    string,
    { selection: string; task: Promise<void> }
  >();
  private retries = new Map<string, { failures: number; after: number }>();
  constructor(
    private readonly ports: DiscoveryPorts,
    private readonly intervalMs = 5000,
    private readonly maxRounds = Infinity,
  ) {}
  cancel(): void {
    this.controller.abort();
  }
  private get canceled(): boolean {
    return this.controller.signal.aborted;
  }
  private wait(ms: number): Promise<void> {
    if (this.canceled) return Promise.resolve();
    return new Promise((resolve) => {
      const done = () => {
        clearTimeout(timer);
        this.controller.signal.removeEventListener("abort", done);
        resolve();
      };
      const timer = setTimeout(done, ms);
      this.controller.signal.addEventListener("abort", done, { once: true });
    });
  }
  async run(): Promise<void> {
    let failures = 0;
    try {
      for (let round = 0; round < this.maxRounds && !this.canceled; round++) {
        try {
          const discovered = await this.ports.discover();
          if (this.canceled) break;
          const candidates = new Map<string, Set<string>>();
          for (const candidate of discovered) {
            if (
              candidate.peerId === this.ports.localPeer ||
              !(Date.parse(candidate.expiresAt) > Date.now())
            )
              continue;
            const routes =
              candidates.get(candidate.peerId) ?? new Set<string>();
            for (const route of candidate.routes) {
              if (
                (route.includes("/wss/") || route.includes("/tls/ws/")) &&
                route.includes("/p2p-circuit/")
              )
                routes.add(route);
            }
            if (routes.size) candidates.set(candidate.peerId, routes);
          }
          if (candidates.size > 16)
            throw Error("Domain exceeds the 16 candidate inspection limit");
          // A missing discovery advertisement is not proof of disconnection. Active streams
          // perform their own bounded liveness/sequence probes, including during DDS outages.
          for (const peer of this.retries.keys()) {
            if (!candidates.has(peer) && !this.active.has(peer))
              this.retries.delete(peer);
          }
          const entries = [...candidates.entries()];
          for (
            let index = 0;
            index < entries.length && !this.canceled;
            index += 4
          ) {
            await Promise.all(
              entries.slice(index, index + 4).map(async ([peer, routes]) => {
                if ((this.retries.get(peer)?.after ?? 0) > Date.now()) return;
                for (const route of [...routes].slice(0, 2)) {
                  if (this.canceled) return;
                  try {
                    const selection = await this.ports.inspect(peer, route);
                    if (this.canceled) return;
                    const current = this.active.get(peer);
                    if (current?.selection === selection) return;
                    if (current) {
                      await this.ports.unfollow(peer);
                      await current.task;
                    }
                    if (!selection || this.canceled) return;
                    if (this.active.size >= 15)
                      throw Error("Session is full (16 peers)");
                    const record = { selection, task: Promise.resolve() };
                    this.active.set(peer, record);
                    record.task = this.ports
                      .follow(selection, peer)
                      .catch(() => {
                        // The transport callback reports the concrete failure per peer.
                      })
                      .finally(() => {
                        if (this.active.get(peer) === record)
                          this.active.delete(peer);
                        const previous = this.retries.get(peer);
                        const count = Math.min(
                          4,
                          (previous?.failures ?? 0) + 1,
                        );
                        this.retries.set(peer, {
                          failures: count,
                          after:
                            Date.now() + this.intervalMs * 2 ** (count - 1),
                        });
                      });
                    return;
                  } catch {
                    /* Try the next route; one unavailable peer does not block others. */
                  }
                }
                const count = Math.min(
                  4,
                  (this.retries.get(peer)?.failures ?? 0) + 1,
                );
                this.retries.set(peer, {
                  failures: count,
                  after: Date.now() + this.intervalMs * 2 ** (count - 1),
                });
              }),
            );
          }
          failures = 0;
          if (!this.canceled)
            this.ports.status(
              this.active.size
                ? `${this.active.size} peer connection${this.active.size === 1 ? "" : "s"} · looking for new arrivals`
                : "Session open · you can map while others join",
            );
        } catch {
          failures = Math.min(4, failures + 1);
          if (!this.canceled)
            this.ports.status(
              "Discovery unavailable · retrying automatically; your map stays open",
            );
        }
        await this.wait(this.intervalMs * 2 ** failures);
      }
    } finally {
      await Promise.all(
        [...this.active.entries()].map(async ([peer, record]) => {
          await this.ports.unfollow(peer);
          await record.task;
        }),
      );
      this.active.clear();
    }
  }
}
