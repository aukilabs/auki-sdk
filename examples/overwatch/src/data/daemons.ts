import { subscribeCluster } from "./cluster";

export type Daemon = {
  /** Browser Overwatch uses the SDK peer id as the stable daemon URL. */
  url: string;
  name: string;
  app: string;
  source: "cluster";
};

type Listener = (daemons: Daemon[]) => void;

let current: Daemon[] = [];
let subscribed = false;
const listeners = new Set<Listener>();

export function getDaemons(): Daemon[] {
  return current;
}

export function subscribeDaemons(cb: Listener): () => void {
  listeners.add(cb);
  cb(current);
  ensureSubscribed();
  return () => listeners.delete(cb);
}

export function findDaemon(url: string): Daemon | undefined {
  return current.find((d) => d.url === url);
}

function ensureSubscribed() {
  if (subscribed) return;
  subscribed = true;
  subscribeCluster((snap) => {
    const next = snap.peers.map((p) => ({
      url: p.peer_id,
      name: p.info.name || p.peer_id,
      app: p.info.app || "unknown",
      source: "cluster" as const,
    }));
    if (sameList(current, next)) return;
    current = next;
    listeners.forEach((cb) => cb(current));
  });
}

function sameList(a: Daemon[], b: Daemon[]): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i += 1) {
    const ai = a[i];
    const bi = b[i];
    if (!ai || !bi) return false;
    if (ai.url !== bi.url || ai.name !== bi.name || ai.app !== bi.app) return false;
  }
  return true;
}
