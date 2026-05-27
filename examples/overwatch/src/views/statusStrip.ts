// StatusStrip — segmented horizontal bar visualizing rolling health
// state over a fixed time window. Used for daemon health timelines on
// directory cards. Each sample lights one slot in the strip; gaps are
// rendered as the "unknown" tone.
//
// Design choices:
// - Fixed slot count, not time-density-scaled — the rightmost slot is
//   "now", oldest on the left. Re-passing samples redraws fully.
// - Status palette only: green / amber / red / grey. Distinct from
//   the orange "live" accent used for identity chrome elsewhere.
// - Hovering a segment surfaces its state name + relative timestamp.

export type HealthState = "ok" | "degraded" | "unreachable" | "unknown";

export type HealthSample = {
  /** Wall-clock ms when the sample was taken. */
  tMs: number;
  state: HealthState;
};

export type StatusStripHandle = {
  el: HTMLElement;
  setSamples(samples: HealthSample[]): void;
};

export type StatusStripOpts = {
  width: number;
  height: number;
  /** Number of slots in the strip. Each slot represents windowMs/slots
   * worth of recent time. */
  slots: number;
  /** Total time window covered by the strip, in ms. */
  windowMs: number;
};

const TONE_COLORS: Record<HealthState, string> = {
  ok: "#22c55e",
  degraded: "#facc15",
  unreachable: "#f87171",
  unknown: "var(--color-rule, #525252)",
};

const TONE_LABELS: Record<HealthState, string> = {
  ok: "healthy",
  degraded: "degraded",
  unreachable: "unreachable",
  unknown: "no signal",
};

export function makeStatusStrip(opts: StatusStripOpts): StatusStripHandle {
  const { width, height, slots, windowMs } = opts;
  const ns = "http://www.w3.org/2000/svg";
  const svg = document.createElementNS(ns, "svg");
  svg.setAttribute("width", String(width));
  svg.setAttribute("height", String(height));
  svg.setAttribute("viewBox", `0 0 ${width} ${height}`);
  svg.setAttribute("preserveAspectRatio", "none");
  svg.style.display = "block";
  svg.style.borderRadius = "1px";

  const slotW = width / slots;
  const rects: SVGRectElement[] = [];
  for (let i = 0; i < slots; i++) {
    const r = document.createElementNS(ns, "rect");
    r.setAttribute("x", (i * slotW).toFixed(2));
    r.setAttribute("y", "0");
    r.setAttribute("width", Math.max(slotW - 0.5, 0.5).toFixed(2));
    r.setAttribute("height", String(height));
    r.setAttribute("fill", TONE_COLORS.unknown);
    r.setAttribute("opacity", "0.35");
    svg.appendChild(r);
    rects.push(r);
  }

  const tooltip = document.createElement("span");
  tooltip.className =
    "pointer-events-none absolute hidden bottom-full mb-1 left-0 -translate-x-1/2 px-1.5 py-0.5 bg-ink-alt border border-paper/15 rounded text-[11px] text-paper/85 whitespace-nowrap z-10";

  const wrap = document.createElement("span");
  wrap.className = "relative inline-block";
  wrap.style.width = `${width}px`;
  wrap.style.height = `${height}px`;
  wrap.appendChild(svg);
  wrap.appendChild(tooltip);

  // Tooltip state, cached so we can re-render on the next mousemove.
  let lastSamples: HealthSample[] = [];
  let lastWindowEnd = Date.now();

  svg.addEventListener("mousemove", (e) => {
    const rect = svg.getBoundingClientRect();
    const x = e.clientX - rect.left;
    const slotIdx = Math.min(slots - 1, Math.max(0, Math.floor(x / slotW)));
    const slotEndMs = lastWindowEnd - (slots - 1 - slotIdx) * (windowMs / slots);
    const slotStartMs = slotEndMs - windowMs / slots;
    const segState = stateForSlot(lastSamples, slotStartMs, slotEndMs);
    const ageMs = Date.now() - slotEndMs;
    tooltip.textContent = `${TONE_LABELS[segState]} · ${formatAge(ageMs)} ago`;
    tooltip.style.left = `${(slotIdx + 0.5) * slotW}px`;
    tooltip.classList.remove("hidden");
  });
  svg.addEventListener("mouseleave", () => {
    tooltip.classList.add("hidden");
  });

  function setSamples(samples: HealthSample[]) {
    lastSamples = samples;
    lastWindowEnd = Date.now();
    const slotMs = windowMs / slots;
    for (let i = 0; i < slots; i++) {
      const slotEndMs = lastWindowEnd - (slots - 1 - i) * slotMs;
      const slotStartMs = slotEndMs - slotMs;
      const state = stateForSlot(samples, slotStartMs, slotEndMs);
      const r = rects[i]!;
      r.setAttribute("fill", TONE_COLORS[state]);
      r.setAttribute("opacity", state === "unknown" ? "0.35" : "0.9");
    }
  }

  return { el: wrap, setSamples };
}

/** Pick the worst-case state for any sample whose tMs falls within
 * [startMs, endMs]. Worst-case so a single failure within the window
 * lights the slot — operators should see flaps, not have them averaged
 * out. Returns "unknown" if no samples land in the slot. */
function stateForSlot(
  samples: HealthSample[],
  startMs: number,
  endMs: number,
): HealthState {
  const PRIORITY: HealthState[] = ["unreachable", "degraded", "ok", "unknown"];
  let worst: HealthState = "unknown";
  let worstRank = PRIORITY.indexOf("unknown");
  for (const s of samples) {
    if (s.tMs < startMs || s.tMs > endMs) continue;
    const rank = PRIORITY.indexOf(s.state);
    if (rank < worstRank) {
      worstRank = rank;
      worst = s.state;
    }
  }
  return worst;
}

function formatAge(ms: number): string {
  if (ms < 1000) return "now";
  const sec = Math.floor(ms / 1000);
  if (sec < 60) return `${sec}s`;
  const min = Math.floor(sec / 60);
  return `${min}m`;
}
