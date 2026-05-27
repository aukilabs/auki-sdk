// Per-peer barrier sync for the camera + pointcloud streams.
//
// ## Why
//
// The K1 publishes `head_left_cam` JPEGs and `stereonet_pointcloud`
// CDR frames on independent producer loops. Without coordination, each
// tile paints its own producer's latest arrival, so a moving subject
// appears at slightly different positions in the two views.
//
// ## How
//
// One synchronization "registry entry" per `(peer_id, source)` pair,
// where `source ∈ {camera, pointcloud}`. Each entry holds:
//   - a reference to the upstream subscription so multiple tiles for
//     the same `(peer, source)` share one libp2p stream
//   - the latest `(timestamp_ns, frame)` from the upstream
//   - the per-listener callbacks waiting for synced output
//
// Dispatch policy on every upstream frame arrival:
//   - For each source that has at least one listener, require its
//     `latest.ts` to be strictly newer than its last-dispatched ts.
//   - If any subscribed source isn't fresh, hold back. Otherwise
//     dispatch the latest from every subscribed source in one pass.
//
// When only one source has listeners (e.g. just the camera tile is
// open), the loop reduces to "this source has a new frame? → dispatch
// immediately". So pass-through is the natural single-source case;
// barrier kicks in automatically when a second tile registers.
//
// `timestamp_ns === 0` (server didn't stamp the response) is treated
// as "always dispatch" so untagged frames don't get stuck behind the
// barrier.
//
// ## Lifecycle
//
// - `subscribeCameraSynced` / `subscribePointCloudSynced` lazily call
//   `subscribePreview` / `subscribePointCloud` on the first listener.
// - When the last listener for a `(peer, source)` unsubscribes, the
//   upstream is torn down. The entry stays in the registry until the
//   peer's other source also goes empty (avoids thrashing on quick
//   tile re-toggles).

import { subscribePreview, type PreviewFrame } from "./preview";
import {
  subscribePointCloud,
  type PointCloudPreviewFrame,
  type PointCloudSpec,
} from "./pointcloudPreview";

type Source = "camera" | "pointcloud";

type Listener = (frame: unknown) => void;

type Entry = {
  /** Most recent `(timestamp_ns, frame)` from upstream. */
  latest: { ts: number; frame: unknown } | null;
  lastDispatchedTs: number;
  listeners: Set<Listener>;
  /** Cleanup for the upstream `subscribePreview` /
   * `subscribePointCloud` subscription. Set on first listener,
   * called when the last listener leaves. */
  upstreamCleanup: (() => void) | null;
};

const peers = new Map<string, Map<Source, Entry>>();

function getInner(peer_id: string): Map<Source, Entry> {
  let inner = peers.get(peer_id);
  if (!inner) {
    inner = new Map();
    peers.set(peer_id, inner);
  }
  return inner;
}

function tryDispatch(peer_id: string): void {
  const inner = peers.get(peer_id);
  if (!inner) return;
  // Only gate on sources that are subscribed AND have produced at
  // least one frame so far. A source with listeners but no frames
  // (just-mounted, not-yet-spawned, or producer-side stalled before
  // any frame ever flowed) doesn't block dispatch — otherwise the
  // healthy stream goes black while we wait on a permanently silent
  // sibling.
  const subscribed: Entry[] = [];
  for (const entry of inner.values()) {
    if (entry.listeners.size > 0 && entry.latest !== null) {
      subscribed.push(entry);
    }
  }
  if (subscribed.length === 0) return;

  // Barrier: every gating source must have advanced past its last
  // dispatched timestamp. Once that holds, dispatch the latest from
  // each source.
  //
  // Tradeoff vs the earlier closest-match design: the camera tile
  // and pointcloud tile may show frames whose timestamps differ by
  // up to one producer interval (~33 ms at 30 Hz), but neither tile
  // is held back to match the other's older content. That keeps the
  // robot-to-rendered latency bounded by the network path rather
  // than by the slower stream's cadence — the closest-match version
  // pushed end-to-end latency near a second when the pointcloud
  // producer dropped to a few Hz.
  for (const entry of subscribed) {
    const latest = entry.latest!;
    if (latest.ts !== 0 && latest.ts <= entry.lastDispatchedTs) return;
  }
  for (const entry of subscribed) {
    const latest = entry.latest!;
    entry.lastDispatchedTs = latest.ts;
    Array.from(entry.listeners).forEach((cb) => {
      try {
        cb(latest.frame);
      } catch (err) {
        // eslint-disable-next-line no-console
        console.error("peerSync listener threw:", err);
      }
    });
  }
}

function ingest(
  peer_id: string,
  source: Source,
  ts: number,
  frame: unknown,
): void {
  const inner = peers.get(peer_id);
  if (!inner) return;
  const entry = inner.get(source);
  if (!entry) return;
  entry.latest = { ts, frame };
  tryDispatch(peer_id);
}

function ensureEntry(
  peer_id: string,
  source: Source,
  sensor_id: string,
): Entry {
  const inner = getInner(peer_id);
  let entry = inner.get(source);
  if (entry) return entry;
  entry = {
    latest: null,
    lastDispatchedTs: -1,
    listeners: new Set(),
    upstreamCleanup: null,
  };
  inner.set(source, entry);
  // Lazily attach the upstream the first time the entry is created.
  // Park's stream cache + grimsby's idle TTL handle reconnection;
  // we only own the JS-side subscription lifetime here.
  if (source === "camera") {
    entry.upstreamCleanup = subscribePreview(
      { peer_id, sensor_id },
      (frame: PreviewFrame | null) => {
        if (!frame) return;
        ingest(peer_id, "camera", frame.timestamp_ns, frame);
      },
    );
  } else {
    entry.upstreamCleanup = subscribePointCloud(
      { peer_id, sensor_id },
      (frame: PointCloudPreviewFrame | null) => {
        if (!frame) return;
        ingest(peer_id, "pointcloud", frame.timestamp_ns, frame);
      },
    );
  }
  return entry;
}

function maybeTeardown(peer_id: string, source: Source): void {
  const inner = peers.get(peer_id);
  if (!inner) return;
  const entry = inner.get(source);
  if (!entry || entry.listeners.size > 0) return;
  entry.upstreamCleanup?.();
  inner.delete(source);
  if (inner.size === 0) peers.delete(peer_id);
}

export function subscribeCameraSynced(
  spec: { peer_id: string; sensor_id: string },
  cb: (frame: PreviewFrame) => void,
): () => void {
  const entry = ensureEntry(spec.peer_id, "camera", spec.sensor_id);
  entry.listeners.add(cb as Listener);
  // Maybe a frame is already buffered; let pass-through fire.
  tryDispatch(spec.peer_id);
  return () => {
    entry.listeners.delete(cb as Listener);
    maybeTeardown(spec.peer_id, "camera");
  };
}

export function subscribePointCloudSynced(
  spec: PointCloudSpec,
  cb: (frame: PointCloudPreviewFrame) => void,
): () => void {
  const entry = ensureEntry(spec.peer_id, "pointcloud", spec.sensor_id);
  entry.listeners.add(cb as Listener);
  tryDispatch(spec.peer_id);
  return () => {
    entry.listeners.delete(cb as Listener);
    maybeTeardown(spec.peer_id, "pointcloud");
  };
}
