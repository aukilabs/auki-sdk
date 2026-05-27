// Per-daemon rolling health history. Piggybacks on the existing
// `/api/info` poller (data/info.ts) — every poll outcome maps to a
// HealthState and is appended to a ring buffer. Subscribers receive
// the buffer on every change, so a StatusStrip view can repaint
// without running its own poller.
//
// Why /api/info and not /api/state:
// - /api/info already has typed outcomes (pending, ok, no_info,
//   unreachable, bad_json) — perfect for the status palette.
// - /api/state's poller only emits on body changes, so a "still
//   reachable" tick doesn't surface. We'd have to add hooks to
//   state.ts; cheaper to use info's poll cadence.
//
// Cadence is 10s (info.ts default) — 30 samples covers a 5-minute
// strip, which is plenty of resolution for "is this device flapping?".

import { subscribeInfo, type InfoStatus } from "./info";
import type { HealthState, HealthSample } from "../views/statusStrip";

const HISTORY_MS = 5 * 60 * 1000;

type Listener = (samples: HealthSample[]) => void;

type Tracker = {
  url: string;
  samples: HealthSample[];
  listeners: Set<Listener>;
  unsubInfo: () => void;
};

const trackers = new Map<string, Tracker>();

export function subscribeHealth(url: string, cb: Listener): () => void {
  let t = trackers.get(url);
  if (!t) {
    const tracker: Tracker = {
      url,
      samples: [],
      listeners: new Set(),
      unsubInfo: () => {},
    };
    tracker.unsubInfo = subscribeInfo(url, (_snap, status) => {
      record(tracker, statusToHealth(status));
    });
    trackers.set(url, tracker);
    t = tracker;
  }
  t.listeners.add(cb);
  cb(t.samples);

  return () => {
    if (!t) return;
    t.listeners.delete(cb);
    if (t.listeners.size === 0) {
      t.unsubInfo();
      trackers.delete(url);
    }
  };
}

function record(t: Tracker, state: HealthState) {
  // "pending" is the initial transient before the first poll lands.
  // We don't record it — strip stays empty (unknown) until the first
  // real sample arrives, which is more honest than backfilling with
  // a misleading "ok" or "unreachable".
  if (state === "unknown") return;
  const now = Date.now();
  t.samples.push({ tMs: now, state });
  // Drop samples older than the rolling window plus a little slack.
  const cutoff = now - HISTORY_MS - 30_000;
  while (t.samples.length > 0 && t.samples[0]!.tMs < cutoff) {
    t.samples.shift();
  }
  t.listeners.forEach((cb) => cb(t.samples));
}

function statusToHealth(status: InfoStatus): HealthState {
  switch (status) {
    case "ok":
      return "ok";
    case "no_info":
    case "bad_json":
      return "degraded";
    case "unreachable":
      return "unreachable";
    case "pending":
      return "unknown";
  }
}
