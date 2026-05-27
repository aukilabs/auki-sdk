// File structure tree — the "what's in this session" panel beneath
// the identity header in the robot sidebar. Live-streaming-only since
// the past-session viewer was retired.
//
// Hybrid layout: top-level branches mirror the on-disk shape the SDK
// writes (`logs/`) so the operator builds an accurate mental model of
// the filesystem; the *leaves* are enriched with semantic data
// (sensor geometry inline, recording duration, clock id) instead of
// raw filenames. Best of both — disk-shape navigation with useful
// labels.
//
// `registry/` used to live as a top-level branch too, but the only
// signal worth surfacing from it (sensor geometry) reads better
// inlined onto each recording's `sensor` leaf. Less nesting, same
// answer.
//
// Source: subscribeInfo + subscribeSensorLogs, plus the sensor
// registry proxy for inline geometry on each recording. Re-renders
// when /api/sensor_logs ticks (recordings list) or /api/info changes,
// and once more when each sensor's geometry resolves.

import { escapeHtml } from "../../util/escape";
import type { Daemon } from "../../data/daemons";
import { subscribeInfo, type InfoSnapshot } from "../../data/info";
import {
  subscribeSensorLogs,
  type DaemonSensorLogs,
  type SensorLog,
} from "../../data/sensorLogs";
import {
  fetchSensorEntry,
  describeSensor,
  shortHash,
  type SensorEntry,
} from "../../data/registry";

export type StructureHandle = {
  el: HTMLElement;
  dispose(): void;
};

export function makeStructure(daemon: Daemon | undefined): StructureHandle {
  const el = document.createElement("div");
  el.className = "px-2 pb-4 flex-1 min-h-0 overflow-y-auto";

  // Subscribed live data.
  let info: InfoSnapshot | null = null;
  let logs: DaemonSensorLogs | null = null;
  // Resolved sensor registry entries — keyed `${id}::${hash}`. Filled
  // lazily as each recording's sensor is fetched; the result is
  // inlined into the recording's "sensor" leaf for an at-a-glance
  // geometry summary (NV12 1920×1080 @ 30fps). Re-render runs each
  // time a fetch resolves.
  const sensorEntries = new Map<string, SensorEntry>();

  // Open/close state for the <details> branches, persisted across
  // repaints. We assume open by default — this Set tracks branches the
  // operator has explicitly closed. Re-rendering would otherwise re-emit
  // the `open` attribute on every poll tick and the user could never
  // keep a branch collapsed.
  const closedKeys = new Set<string>();

  // Why click delegation, not the toggle event:
  //
  //   The `toggle` event on <details> does NOT bubble (HTML spec —
  //   bubbles: false), so a delegated listener on `el` never fires for
  //   descendants. `click` bubbles fine and only fires on actual user
  //   interaction (including keyboard activation, since that dispatches
  //   a click too). The browser hasn't toggled the <details> yet at
  //   click time, so we predict the post-click state with
  //   `!details.open`.
  el.addEventListener("click", (ev) => {
    const target = ev.target as HTMLElement | null;
    const summary = target?.closest("summary");
    if (!summary) return;
    const details = summary.parentElement;
    if (!details || details.tagName !== "DETAILS") return;
    const key = details.getAttribute("data-key");
    if (!key) return;
    const willBeOpen = !(details as HTMLDetailsElement).open;
    if (willBeOpen) closedKeys.delete(key);
    else closedKeys.add(key);
  });

  /** Walk the freshly-rendered tree and re-apply persisted closed
   * state. Cheap (small DOM) — runs after every repaint. We touch the
   * `open` attribute directly rather than the property so we don't
   * have to worry about the toggle event firing here. */
  const applyClosedState = () => {
    el.querySelectorAll<HTMLDetailsElement>("details[data-key]").forEach((d) => {
      const key = d.getAttribute("data-key");
      if (key && closedKeys.has(key)) {
        d.removeAttribute("open");
      }
    });
  };

  const repaint = () => {
    if (!daemon) {
      el.innerHTML = emptyShell("No device — nothing to inspect.");
      return;
    }
    el.innerHTML = renderLive(info, logs, sensorEntries);
    ensureLiveRegistryFetches();
    applyClosedState();
  };

  /** Walk the live state's sensor logs and queue sensor geometry
   * fetches for any sensor (id, hash) we haven't seen yet. Each fetch
   * re-paints when it lands so the geometry note in the log's
   * sensor leaf materialises without polling. */
  const ensureLiveRegistryFetches = () => {
    if (!daemon || !logs) return;
    const seen = new Set<string>();
    for (const l of logs.sensor_logs) {
      const key = `${l.sensor_id}::${l.sensor_hash}`;
      if (sensorEntries.has(key) || seen.has(key)) continue;
      seen.add(key);
      void fetchSensorEntry(daemon.url, l.sensor_id, l.sensor_hash).then(
        (entry) => {
          if (entry) {
            sensorEntries.set(key, entry);
            repaint();
          }
        },
      );
    }
  };

  let disposeInfo: (() => void) | null = null;
  let disposeLogs: (() => void) | null = null;
  if (daemon) {
    disposeInfo = subscribeInfo(daemon.url, (snap) => {
      info = snap;
      repaint();
    });
    disposeLogs = subscribeSensorLogs(daemon.url, (s) => {
      logs = s;
      repaint();
    });
  }

  repaint();

  return {
    el,
    dispose() {
      disposeInfo?.();
      disposeLogs?.();
    },
  };
}

// ─── render ──────────────────────────────────────────────────────────────────

function emptyShell(msg: string): string {
  return `<div class="px-3 py-4 text-rule/60 text-xs italic">${escapeHtml(msg)}</div>`;
}

function renderLive(
  info: InfoSnapshot | null,
  state: DaemonSensorLogs | null,
  sensorEntries: Map<string, SensorEntry>,
): string {
  if (!info && !state) {
    return emptyShell("Waiting for daemon — no live data yet.");
  }

  const logs = state?.sensor_logs ?? [];
  const active = logs.filter((l) => l.stopped_at_ns == null);
  const stopped = logs.filter((l) => l.stopped_at_ns != null);

  return folder("live/logs", "Logs", "active runtime", [
    folder(
      "live/logs/sensor_logs",
      "Sensor logs",
      logs.length === 0
        ? "none yet"
        : `${active.length} active · ${stopped.length} stopped`,
      logs.length === 0
        ? [emptyLeaf("no logs yet — daemon hasn't opened any auto-buffers")]
        : logs.map((l) => sensorLogBranch(l, sensorEntries)),
    ),
  ]);
}

// ─── tree primitives ─────────────────────────────────────────────────────────
//
// Visual contract — designed to read like a traditional file/dropdown
// list, not a CLI tree:
//
//   • Branches are sentence-case sans-serif by default (Logs,
//     Recordings, Sensors). Pass `{ mono: true }` for branches whose
//     name is itself an identifier (recording IDs, sensor IDs).
//   • Leaves default to monospace because their names are filenames or
//     field labels (manifest.json, segments/, sensor_hash).
//   • Names are at left position 28px from the parent's content edge
//     (px-2 + chevron 12px + gap-2 = 28px). Children get pl-5 (20px)
//     so nested branches and leaves align directly under the parent
//     name above them — chevron-aligned indentation, like Finder.
//   • Hover: subtle bg-paper/5 across the whole row. Rows are py-1.5
//     for a comfortable click target.
//   • Notes (the dim subtitle: count, geometry, status) read after a
//     1.5-unit gap from the name in muted rule colour — same idea as
//     a file size column in a list view, but inline because we don't
//     have grid columns.

const CHEVRON_SVG = `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" class="w-3 h-3 text-rule/60 shrink-0 transition-transform duration-150 group-open/branch:rotate-90"><polyline points="9 6 15 12 9 18"></polyline></svg>`;

/** A collapsible folder. Uses native <details> for accessibility +
 * zero-JS open state. `key` is a stable identifier the parent uses to
 * persist open/closed state across repaints. `note` is the dim
 * subtitle. `mono` switches the name to a monospace font for ID-style
 * labels. */
function folder(
  key: string,
  name: string,
  note: string,
  children: string[],
  opts: { open?: boolean; mono?: boolean } = {},
): string {
  const openAttr = opts.open !== false ? " open" : "";
  const nameClass = opts.mono
    ? "font-mono text-paper/85 text-[12px] truncate"
    : "text-paper/90 text-xs truncate";
  return `<details${openAttr} data-key="${escapeHtml(key)}" class="group/branch">
      <summary class="flex items-center gap-2 px-2 py-1.5 rounded-md hover:bg-paper/5 cursor-pointer select-none list-none">
        ${CHEVRON_SVG}
        <span class="${nameClass}" title="${escapeHtml(name)}">${escapeHtml(name)}</span>
        ${note ? `<span class="text-rule/55 text-[12px] truncate ml-1.5">${escapeHtml(note)}</span>` : ""}
      </summary>
      <div class="pl-5">${children.join("")}</div>
    </details>`;
}

/** A non-expandable leaf — file or field. Name is monospace by
 * default (these are real filenames and field identifiers). */
function leaf(
  name: string,
  note: string,
  opts: { mono?: boolean } = {},
): string {
  const mono = opts.mono ?? true;
  const nameClass = mono
    ? "font-mono text-paper/75 text-[12px] truncate"
    : "text-paper/80 text-xs truncate";
  return `<div class="flex items-baseline gap-2 px-2 py-1 rounded-md hover:bg-paper/5">
      <span class="${nameClass}" title="${escapeHtml(name)}">${escapeHtml(name)}</span>
      ${note ? `<span class="text-rule/55 text-[12px] truncate ml-1.5">${escapeHtml(note)}</span>` : ""}
    </div>`;
}

/** Shown inside an empty folder so it doesn't collapse to nothing. */
function emptyLeaf(msg: string): string {
  return `<div class="px-2 py-1 text-rule/45 italic text-[12px]">${escapeHtml(msg)}</div>`;
}

function sensorLogBranch(
  l: SensorLog,
  sensorEntries: Map<string, SensorEntry>,
): string {
  const isBuffer = l.retention_ns > 0;
  const isActive = l.stopped_at_ns == null;
  const kind = isBuffer ? "buffer" : "recording";
  const retention = isBuffer
    ? `${formatNs(l.retention_ns)} ring`
    : "unbounded";
  // duration_ns under Control API v1 is the configured forward cap,
  // not the elapsed extent. Surface it as "cap N" when set; the
  // captured-extent value is only meaningful once the log stopped.
  const cap = l.duration_ns > 0 ? `cap ${formatNs(l.duration_ns)}` : null;
  const status = isActive
    ? "active"
    : `stopped · ${formatNs((l.stopped_at_ns ?? 0) - l.started_at_ns)} captured`;
  const note = [kind, retention, cap, status].filter(Boolean).join(" · ");

  // Sensor label: name · geometry, where geometry is enriched in
  // place once the registry fetch resolves (otherwise "loading
  // geometry…" appears, replaced by a real value within a poll tick).
  const sensor = shortenSensorId(l.sensor_id);
  const sensorEntry = sensorEntries.get(`${l.sensor_id}::${l.sensor_hash}`);
  const geometry = sensorEntry
    ? ` · ${describeSensor(sensorEntry)}`
    : " · loading geometry…";
  const sensorLabel = `${sensor}${geometry}`;

  return folder(
    `live/logs/sensor_logs/${l.sensor_log_id}`,
    l.sensor_log_id,
    note,
    [
      leaf("sensor", sensorLabel),
      leaf("sensor_hash", shortHash(l.sensor_hash)),
      leaf("clock", shortenSensorId(l.clock_id)),
    ],
    { mono: true },
  );
}

// ─── helpers ─────────────────────────────────────────────────────────────────

/** Sensor / clock IDs are namespaced (`K1-AABBCCDDEEFF/head_left_cam`).
 * The full ID lands in `title` for hover; the on-screen label drops
 * the device prefix to keep the tree narrow. */
function shortenSensorId(id: string): string {
  const slash = id.lastIndexOf("/");
  if (slash < 0) return id;
  return id.slice(slash + 1);
}

/** Format a nanosecond duration into a compact human label. */
function formatNs(ns: number): string {
  const ms = ns / 1_000_000;
  if (ms < 1_000) return `${Math.round(ms)}ms`;
  const s = ms / 1_000;
  if (s < 60) return `${s.toFixed(s < 10 ? 1 : 0)}s`;
  const m = Math.floor(s / 60);
  const rs = Math.floor(s % 60);
  return rs === 0 ? `${m}m` : `${m}m${rs}s`;
}
