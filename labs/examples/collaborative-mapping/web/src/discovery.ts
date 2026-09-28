/** Domain discovery is a candidate source; inspect authenticates routes and matches session metadata. */
export interface Candidate {
  peerId: string;
  routes: string[];
  expiresAt: string;
}
export interface Match {
  peerId: string;
  selection: string;
}
export interface DiscoveryPorts {
  localPeer: string;
  discover(): Promise<Candidate[]>;
  inspect(peer: string, route: string): Promise<string | undefined>;
  follow(selection: string): Promise<void>;
  matches(matches: Match[]): void;
  status(message: string): void;
}
export class DomainDiscovery {
  private readonly controller = new AbortController();
  private selected: string | undefined;
  constructor(
    private readonly ports: DiscoveryPorts,
    private readonly intervalMs = 5000,
    private readonly maxRounds = Number.POSITIVE_INFINITY,
  ) {}
  choose(peer: string): void {
    this.selected = peer || undefined;
  }
  cancel(): void {
    this.controller.abort();
  }
  private get canceled(): boolean {
    return this.controller.signal.aborted;
  }
  private wait(): Promise<void> {
    if (this.canceled) return Promise.resolve();
    return new Promise((resolve) => {
      const done = () => {
        clearTimeout(timer);
        this.controller.signal.removeEventListener("abort", done);
        resolve();
      };
      const timer = setTimeout(done, this.intervalMs);
      this.controller.signal.addEventListener("abort", done, { once: true });
    });
  }
  async run(): Promise<void> {
    let failures = 0;
    for (let round = 0; round < this.maxRounds && !this.canceled; round++) {
      try {
        this.ports.status(
          round
            ? "Refreshing Domain discovery · waiting for a matching session"
            : "Finding peers in this Domain and demo session…",
        );
        const discovered = await this.ports.discover();
        if (this.canceled) return;
        // Deduplicate advertisements; never auto-select from a silently truncated candidate list.
        const candidates = new Map<string, Set<string>>();
        for (const candidate of discovered) {
          if (
            candidate.peerId === this.ports.localPeer ||
            !(Date.parse(candidate.expiresAt) > Date.now())
          )
            continue;
          const routes = candidates.get(candidate.peerId) ?? new Set<string>();
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
          throw new Error(
            "Too many component peers in this Domain to inspect safely (limit 16).",
          );
        const matches: Match[] = [];
        let unavailable = 0;
        // Bound concurrent Catalog requests to four. One stale advertisement must not block the rest.
        const entries = [...candidates.entries()];
        for (
          let index = 0;
          index < entries.length && !this.canceled;
          index += 4
        ) {
          await Promise.all(
            entries.slice(index, index + 4).map(async ([peerId, routes]) => {
              let inspected = false;
              for (const route of [...routes].slice(0, 2)) {
                if (this.canceled) return;
                try {
                  const selection = await this.ports.inspect(peerId, route);
                  inspected = true;
                  if (!this.canceled && selection)
                    matches.push({ peerId, selection });
                  break;
                } catch {
                  /* Try the candidate's next WSS route. */
                }
              }
              if (!inspected) unavailable++;
            }),
          );
        }
        if (this.canceled) return;
        matches.sort((a, b) => a.peerId.localeCompare(b.peerId));
        this.ports.matches(matches);
        const selected = this.selected
          ? matches.find((match) => match.peerId === this.selected)
          : matches.length === 1
            ? matches[0]
            : undefined;
        if (selected) {
          this.ports.status("Matching peer found · connecting both map views");
          await this.ports.follow(selected.selection);
          if (this.canceled) return;
          throw new Error("Partner subscription ended.");
        }
        if (!matches.length && unavailable) {
          throw new Error(
            "Matching sessions could not be checked because some peers are unreachable.",
          );
        }
        failures = 0;
        this.ports.status(
          matches.length > 1
            ? "Several peers share this session · choose your partner below"
            : unavailable
              ? "Some peers are unreachable · retrying Domain discovery"
              : this.selected
                ? "Selected partner unavailable · waiting for it to return"
                : "Waiting for another peer in this Domain and demo session",
        );
      } catch (error) {
        if (this.canceled) return;
        failures++;
        this.ports.status(
          `Discovery / connection failed · remote map may be stale (${failures}/3): ${error instanceof Error ? error.message : String(error)}`,
        );
        if (failures >= 3)
          throw new Error(
            "Automatic connection paused after three failures. Click Retry discovery.",
          );
      }
      await this.wait();
    }
    if (!this.canceled)
      throw new Error(
        "Discovery search stopped. Check the Domain and session, then retry discovery.",
      );
  }
}
