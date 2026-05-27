// Live video tile — subscribes to a per-daemon shared frame source
// (data/preview.ts) and renders incoming JPEG frames into an <img>.
// Uses the shared `makeTileChrome` shell so close button + identity
// chip + bottom bar are pixel-identical to every other tile type.
//
// Per-tile controls (live):
//   • Freeze button   (pauses frame source for this tile only)
//
// Recording controls (Record button + "Live ▾" source dropdown) used
// to live here. Both were removed: the dropdown only ever offered one
// option, and the button was wired to Park-side proxy handlers that
// no longer exist. Track any future recording-control UX decision on
// the project board once the SDK side is ready.
//
// Until the SDK exposes per-sensor preview endpoints (ARCHITECTURE §6)
// every live video tile from the same daemon shows the same frame.

import { escapeHtml } from "../../../util/escape";
import {
  iconCamera,
  iconForSensorType,
  iconFreeze,
  iconInfo,
  sensorTypeFromEntry,
} from "../../../icons";
import {
  subscribePreview,
  getStreamState,
  type PreviewFrame,
  type StreamSpec,
} from "../../../data/preview";
import {
  openInspector,
  type InspectorContent,
  type InspectorHandle,
} from "../../../shell/inspectorDrawer";
import { makeSparkline, type SparklineTone } from "../../sparkline";
import { captureLiveFrame } from "../screenshot";
import { shortPeer } from "../../../data/cluster";
import type { SensorEntry } from "../../../data/registry";
import { makeChromeBtn, makeTileChrome } from "./chrome";
import { shortName } from "./names";
import type { TileHandle } from "../tile";
import { makeRecordingInspectorControl } from "../recordingControl";

// Threshold for the "no signal" overlay. Pre-dedup polling masked
// short producer stalls (the browser painted the same cached frame at
// hundreds of fps so `lastFrameAt` looked alive), so a 3 s threshold
// felt sensitive enough. Post-dedup the threshold reflects real
// no-new-frame intervals; bumped to 5 s to absorb the K1's occasional
// brief hiccups without flashing the overlay.
const STALL_MS = 5000;
const FPS_UPDATE_MS = 1000;
/** Rolling buffer of inter-frame intervals (ms) feeding the bottom-bar
 * sparkline + the inspector's quality panel. ~60 samples = 2s at 30fps,
 * 12s at 5fps — enough to see jitter without burying single drops. */
const INTERVAL_BUFFER_LEN = 60;

export function makeLiveVideoTile(
  spec: {
    sensor_id: string;
    sensor_hash?: string;
    daemon_url: string;
    peer_id: string;
    entry?: SensorEntry;
  },
  opts: { onClose: () => void },
): TileHandle {
  const chrome = makeTileChrome({
    sensor_id: spec.sensor_id,
    entry: spec.entry,
    onClose: opts.onClose,
  });
  const el = chrome.el;

  // ─── body: live preview <img> ──────────────────────────────────────
  const img = document.createElement("img");
  img.className = "absolute inset-0 w-full h-full object-contain bg-ink";
  img.alt = `${spec.sensor_id} live preview`;
  chrome.body.appendChild(img);

  // (No top-left chip beyond the sensor identity badge — the
  // recording-source dropdown lived here once; see header comment.)

  // ─── bottom info ───────────────────────────────────────────────────
  // Layout: sensor_id · fps · sparkline · latency · drops
  // Sparkline rides between fps and latency so it visually anchors to
  // the rate it's describing. Tone tracks jitter (see updateSparkline).
  chrome.bottomInfo.innerHTML = `
    <span class="truncate" title="${escapeHtml(spec.sensor_id)}">${escapeHtml(spec.sensor_id)}</span>
    <span class="text-rule/70 shrink-0" data-region="fps">— fps</span>
    <span class="shrink-0 opacity-70" data-region="sparkline" title="Frame interval over the last ~${INTERVAL_BUFFER_LEN} frames"></span>
    <span class="text-rule/70 shrink-0 font-mono tabular-nums" data-region="latency" title="Age of the last frame when Park served it">—</span>
    <span class="text-red-400/80 shrink-0 hidden" data-region="drops" title="Dropped frames detected (seq gaps)">0 dropped</span>
  `;

  const sparkline = makeSparkline({
    width: 56,
    height: 12,
    tone: "muted",
  });
  const sparklineSlot = chrome.bottomInfo.querySelector(
    '[data-region="sparkline"]',
  ) as HTMLElement;
  sparklineSlot.appendChild(sparkline.el);

  // ─── bottom actions: freeze + snapshot + inspector ────────────────
  const freezeBtn = makeChromeBtn(iconFreeze(13), "Freeze frame (F)");
  const snapBtn = makeChromeBtn(iconCamera(13), "Save frame as PNG (S)");
  const inspectBtn = makeChromeBtn(iconInfo(13), "Inspect frame");
  chrome.bottomActions.append(freezeBtn, snapBtn, inspectBtn);

  // ─── stall overlay (covers body when no frames arrive) ────────────
  const stallOverlay = document.createElement("div");
  stallOverlay.className =
    "absolute inset-0 hidden flex-col items-center justify-center bg-ink/70 backdrop-blur-[2px] gap-1 text-rule pointer-events-none";
  stallOverlay.innerHTML = `
    <span class="text-[11px] uppercase tracking-[0.2em] text-paper/70" data-region="stall-title">no signal</span>
    <span class="text-[11px] text-rule/70" data-region="stall-age">—</span>
  `;
  chrome.body.appendChild(stallOverlay);
  const stallTitleEl = stallOverlay.querySelector('[data-region="stall-title"]') as HTMLElement;
  const stallAgeEl = stallOverlay.querySelector('[data-region="stall-age"]') as HTMLElement;

  const fpsEl = chrome.bottomInfo.querySelector('[data-region="fps"]') as HTMLElement;
  const latencyEl = chrome.bottomInfo.querySelector('[data-region="latency"]') as HTMLElement;
  const dropsEl = chrome.bottomInfo.querySelector('[data-region="drops"]') as HTMLElement;

  // ─── state + frame chain ──────────────────────────────────────────
  const streamSpec: StreamSpec = { peer_id: spec.peer_id, sensor_id: spec.sensor_id };
  let unloaded = false;
  let frozen = false;
  let lastFrameAt: number | null = null;
  const frameTimes: number[] = [];
  /** Inter-frame intervals (ms) — newest at the end. Sparkline + jitter
   * stats read from this; capped at INTERVAL_BUFFER_LEN. */
  const intervalsMs: number[] = [];
  let lastFps: number | null = null;
  let stalled = false;
  let lastFpsUpdate = 0;
  let lastSeq: number | null = null;
  let totalDrops = 0;
  let lastFrame: PreviewFrame | null = null;
  let inspector: InspectorHandle | null = null;
  const recordingControl = makeRecordingInspectorControl({
    peerId: spec.peer_id,
    sensorId: spec.sensor_id,
    onChange: () => {
      if (inspector?.isOpen()) inspector.update(buildInspectorContent());
    },
  });

  const setStalled = (s: boolean) => {
    if (stalled === s) return;
    stalled = s;
    if (s) {
      stallOverlay.classList.remove("hidden");
      stallOverlay.classList.add("flex");
      el.classList.add("opacity-90");
    } else {
      stallOverlay.classList.add("hidden");
      stallOverlay.classList.remove("flex");
      el.classList.remove("opacity-90");
    }
  };

  const stallTimer = window.setInterval(() => {
    if (unloaded || frozen) return;
    const state = getStreamState(streamSpec);
    if (lastFrameAt == null) {
      // No frames ever — show the typed stream state.
      if (state !== "live") {
        setStalled(true);
        stallTitleEl.textContent = stallTitleForState(state);
        stallAgeEl.textContent = "—";
      }
      return;
    }
    const age = performance.now() - lastFrameAt;
    if (age > STALL_MS) {
      setStalled(true);
      stallTitleEl.textContent = stallTitleForState(state);
      stallAgeEl.textContent = `last frame ${formatStallAge(age)} ago`;
    }
  }, 1000);

  const onFrame = async (frame: PreviewFrame | null) => {
    if (unloaded || frozen || !frame) return;

    // Gate on `seq` — Park's preview poller (data/preview.ts) is
    // pipelined and re-fetches the cached JPEG hundreds of times per
    // second when the server is fast. Without this gate, FPS measures
    // poll rate (300+), latency redraws on every poll (unreadable),
    // and the sparkline samples poll-to-poll intervals (~3ms jitter).
    // Skip duplicate-seq responses so the rest of the handler only
    // sees genuinely new frames at the producer's actual frame rate.
    if (lastSeq !== null && frame.seq === lastSeq) return;

    const loader = new Image();
    loader.src = frame.url;
    try {
      await loader.decode();
    } catch {
      // ignore decode errors
    }
    if (unloaded || frozen) return;
    img.src = frame.url;
    const prevFrameAt = lastFrameAt;
    lastFrameAt = performance.now();
    lastFrame = frame;
    setStalled(false);

    // Frame age: how stale was the cached frame when Park served it.
    latencyEl.textContent = frame.frameAgeMs < 1000
      ? `${frame.frameAgeMs}ms`
      : `${(frame.frameAgeMs / 1000).toFixed(1)}s`;

    // Drop detection via seq gaps. seq < lastSeq means producer reset
    // (reconnect) — don't count those bytes as drops.
    if (lastSeq !== null && frame.seq > lastSeq + 1) {
      totalDrops += frame.seq - lastSeq - 1;
    } else if (lastSeq !== null && frame.seq < lastSeq) {
      totalDrops = 0;
    }
    lastSeq = frame.seq;
    if (totalDrops > 0) {
      dropsEl.textContent = `${totalDrops} dropped`;
      dropsEl.classList.remove("hidden");
    } else {
      dropsEl.classList.add("hidden");
    }

    frameTimes.push(lastFrameAt);
    while (frameTimes.length > 0 && lastFrameAt - frameTimes[0]! > 2000) {
      frameTimes.shift();
    }
    if (lastFrameAt - lastFpsUpdate > FPS_UPDATE_MS && frameTimes.length >= 2) {
      const span = (lastFrameAt - frameTimes[0]!) / 1000;
      const fps = (frameTimes.length - 1) / Math.max(span, 0.001);
      lastFps = fps;
      fpsEl.textContent = `${fps.toFixed(1)} fps`;
      lastFpsUpdate = lastFrameAt;
    }

    // Inter-frame interval into the rolling sparkline buffer. Skip the
    // very first frame (no prior to subtract) and any "interval" that
    // would imply a stall recovery — those should land in the buffer
    // honestly so the spike is visible, not smoothed away.
    if (prevFrameAt != null) {
      intervalsMs.push(lastFrameAt - prevFrameAt);
      while (intervalsMs.length > INTERVAL_BUFFER_LEN) intervalsMs.shift();
      updateSparkline();
    }

    // Inspector content refreshes whenever the drawer is pinned to
    // this tile. Cheap — render is rAF-batched inside the drawer.
    if (inspector?.isOpen()) {
      inspector.update(buildInspectorContent());
    }
  };

  function updateSparkline() {
    if (intervalsMs.length < 2) return;
    sparkline.setValues(intervalsMs);
    const mean = intervalsMs.reduce((a, b) => a + b, 0) / intervalsMs.length;
    const variance =
      intervalsMs.reduce((a, b) => a + (b - mean) ** 2, 0) / intervalsMs.length;
    const cv = Math.sqrt(variance) / Math.max(mean, 1);
    let tone: SparklineTone = "ok";
    let toneLabel = "steady";
    if (cv > 0.4) { tone = "bad"; toneLabel = "very irregular"; }
    else if (cv > 0.15) { tone = "warn"; toneLabel = "some jitter"; }
    sparkline.setTone(tone);
    sparkline.el.title =
      `Frame-to-frame interval over the last ${intervalsMs.length} frames\n` +
      `mean ${mean.toFixed(0)}ms · jitter (CV) ${(cv * 100).toFixed(0)}% · ${toneLabel}\n` +
      `orange ≤15% · yellow 15–40% · red >40%`;
  }

  function buildInspectorContent(): InspectorContent {
    const state = getStreamState(streamSpec);
    const f = lastFrame;
    const stateLabel = (() => {
      if (state === "live") return { label: "LIVE", tone: "live" as const };
      if (state === "connecting" || state === "reconnecting") {
        return { label: state.toUpperCase(), tone: "warn" as const };
      }
      if (state === "declined") return { label: "DECLINED", tone: "warn" as const };
      if (state === "rejected") return { label: "REJECTED", tone: "warn" as const };
      return { label: "OFFLINE", tone: "muted" as const };
    })();

    let cv = 0;
    let meanInterval = 0;
    if (intervalsMs.length >= 2) {
      meanInterval =
        intervalsMs.reduce((a, b) => a + b, 0) / intervalsMs.length;
      const variance =
        intervalsMs.reduce((a, b) => a + (b - meanInterval) ** 2, 0) /
        intervalsMs.length;
      cv = Math.sqrt(variance) / Math.max(meanInterval, 1);
    }

    return {
      title: shortName(spec.sensor_id),
      subtitle: spec.daemon_url,
      badge: stateLabel,
      actions: recordingControl.actions(),
      sections: [
        recordingControl.section(),
        {
          title: "Frame",
          rows: f
            ? [
                { key: "seq", value: String(f.seq) },
                {
                  key: "age",
                  value:
                    f.frameAgeMs < 1000
                      ? `${f.frameAgeMs}ms`
                      : `${(f.frameAgeMs / 1000).toFixed(2)}s`,
                },
                {
                  key: "received",
                  value: new Date(f.receivedAtWallMs).toISOString(),
                },
                { key: "bytes", value: formatBytes(f.bytes) },
              ]
            : [{ key: "status", value: "no frame yet", dim: true, mono: false }],
        },
        {
          title: "Sensor",
          rows: [
            { key: "sensor_id", value: spec.sensor_id },
            {
              key: "sensor_hash",
              value: spec.sensor_hash ?? f?.sensorHash ?? "—",
              dim: !(spec.sensor_hash || f?.sensorHash),
            },
          ],
        },
        {
          title: "Stream",
          rows: [
            {
              key: "state",
              value: state,
              mono: false,
            },
            {
              key: "peer_id",
              html: true,
              value: `<span title="${escapeHtml(spec.peer_id)}">${escapeHtml(shortPeer(spec.peer_id))}</span>`,
              copy: spec.peer_id,
            },
            {
              key: "clock_id",
              value: f?.clockId ?? "—",
              dim: !f?.clockId,
            },
          ],
        },
        {
          title: "Quality",
          rows: [
            {
              key: "fps",
              value: lastFps != null ? lastFps.toFixed(1) : "—",
              dim: lastFps == null,
            },
            {
              key: "interval",
              value: meanInterval > 0 ? `${meanInterval.toFixed(0)}ms` : "—",
              dim: meanInterval === 0,
            },
            {
              key: "jitter (cv)",
              value: meanInterval > 0 ? cv.toFixed(2) : "—",
              dim: meanInterval === 0,
            },
            {
              key: "drops",
              value: String(totalDrops),
              dim: totalDrops === 0,
            },
            {
              key: "samples",
              value: String(intervalsMs.length),
              dim: intervalsMs.length === 0,
            },
          ],
        },
      ],
    };
  }

  function openTileInspector() {
    inspector = openInspector(buildInspectorContent());
  }
  inspectBtn.addEventListener("click", () => openTileInspector());

  function takeSnapshot() {
    captureLiveFrame(img, {
      sensorId: spec.sensor_id,
      daemonUrl: spec.daemon_url,
      peerId: spec.peer_id,
      receivedAtWallMs: lastFrame?.receivedAtWallMs,
      seq: lastFrame?.seq,
    });
  }
  snapBtn.addEventListener("click", () => takeSnapshot());

  el.addEventListener("keydown", (e) => {
    if (e.key === "i" && !e.metaKey && !e.ctrlKey && !e.altKey) {
      const t = e.target as HTMLElement | null;
      if (t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA")) return;
      e.preventDefault();
      openTileInspector();
    }
  });

  const unsubscribe = subscribePreview(
    { peer_id: spec.peer_id, sensor_id: spec.sensor_id },
    onFrame,
  );

  const handle: TileHandle = {
    el,
    dispose() {
      unloaded = true;
      unsubscribe();
      clearInterval(stallTimer);
      recordingControl.dispose();
      // If we own the open inspector, close it so it doesn't linger
      // showing data from a tile that's just been unmounted.
      inspector?.close();
      inspector = null;
    },
    toggleFreeze() {
      frozen = !frozen;
      freezeBtn.classList.toggle("text-accent", frozen);
      freezeBtn.title = frozen ? "Unfreeze (F)" : "Freeze frame (F)";
      chrome.setChipState(
        frozen
          ? {
              label: "Frozen",
              iconHtml: iconFreeze(12),
              tone: "accent",
            }
          : {
              label: shortName(spec.sensor_id),
              iconHtml: iconForSensorType(sensorTypeFromEntry(spec.sensor_id, spec.entry), 14),
              tone: "default",
            },
      );
    },
    snapshot() {
      takeSnapshot();
    },
    close() {
      opts.onClose();
    },
    isFrozen() {
      return frozen;
    },
    sensorId() {
      return spec.sensor_id;
    },
    setSensorLogs() {},
  };

  freezeBtn.addEventListener("click", () => handle.toggleFreeze());

  return handle;
}

// ─── private helpers ────────────────────────────────────────────────────────

/** Stall-overlay age formatter. Second precision (`"5s"` / `"1m30s"`). */
function formatStallAge(ms: number): string {
  const sec = Math.floor(ms / 1000);
  if (sec < 60) return `${sec}s`;
  const min = Math.floor(sec / 60);
  return `${min}m${(sec % 60).toString().padStart(2, "0")}s`;
}

/** Bytes → human-readable. Used in the inspector's Frame panel. */
function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / (1024 * 1024)).toFixed(2)} MB`;
}

function stallTitleForState(state: string): string {
  if (state === "connecting") return "connecting…";
  if (state === "reconnecting") return "reconnecting…";
  if (state === "declined") return "stream declined";
  if (state === "rejected") return "stream rejected by peer";
  return "no signal";
}
