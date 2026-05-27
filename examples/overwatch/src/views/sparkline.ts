// Tiny SVG sparkline. Renders a single polyline over a fixed-size
// rolling buffer. Used today for the live tile's frame-interval
// quality strip; designed generic so it can show any 1D series.
//
// Implementation notes:
// - Caller pushes raw values; the component scales to its own min/max.
// - Optional baseline (e.g. expected frame interval) draws as a faint
//   horizontal rule so the eye can compare jitter against the target.
// - Tone is set per-update via setTone() so the host (live tile) can
//   recolor when fps drops below threshold without re-rendering the
//   whole element.
//
// No animation, no smoothing — values land on the polyline as they
// arrive. We're showing jitter, not hiding it.

export type SparklineTone = "ok" | "warn" | "bad" | "muted";

export type SparklineHandle = {
  el: HTMLElement;
  /** Replace the data series. Component scales to its own min/max. */
  setValues(values: number[]): void;
  /** Draw a horizontal rule at this value (in source units). null hides it. */
  setBaseline(value: number | null): void;
  /** Update the line color according to the host's quality assessment. */
  setTone(tone: SparklineTone): void;
};

export type SparklineOpts = {
  width: number;
  height: number;
  /** Optional initial baseline (source units, e.g. expected interval ms). */
  baseline?: number | null;
  /** Optional initial tone. Defaults to "ok". */
  tone?: SparklineTone;
};

const TONE_COLORS: Record<SparklineTone, string> = {
  ok: "var(--color-accent, #f97316)",
  warn: "#facc15",
  bad: "#f87171",
  muted: "var(--color-rule, #525252)",
};

export function makeSparkline(opts: SparklineOpts): SparklineHandle {
  const { width, height } = opts;
  const ns = "http://www.w3.org/2000/svg";
  const svg = document.createElementNS(ns, "svg");
  svg.setAttribute("width", String(width));
  svg.setAttribute("height", String(height));
  svg.setAttribute("viewBox", `0 0 ${width} ${height}`);
  svg.setAttribute("preserveAspectRatio", "none");
  svg.setAttribute("aria-hidden", "true");
  svg.style.display = "block";
  svg.style.flex = "0 0 auto";

  const baselineEl = document.createElementNS(ns, "line");
  baselineEl.setAttribute("x1", "0");
  baselineEl.setAttribute("x2", String(width));
  baselineEl.setAttribute("stroke", "var(--color-rule, #525252)");
  baselineEl.setAttribute("stroke-width", "1");
  baselineEl.setAttribute("stroke-dasharray", "2 2");
  baselineEl.setAttribute("opacity", "0.35");
  baselineEl.style.display = "none";
  svg.appendChild(baselineEl);

  const path = document.createElementNS(ns, "polyline");
  path.setAttribute("fill", "none");
  path.setAttribute("stroke-width", "1.25");
  path.setAttribute("stroke-linejoin", "round");
  path.setAttribute("stroke-linecap", "round");
  svg.appendChild(path);

  const wrap = document.createElement("span");
  wrap.style.display = "inline-flex";
  wrap.style.alignItems = "center";
  wrap.appendChild(svg);

  let values: number[] = [];
  let baseline: number | null = opts.baseline ?? null;
  let tone: SparklineTone = opts.tone ?? "ok";
  applyTone();

  function applyTone() {
    path.setAttribute("stroke", TONE_COLORS[tone]);
  }

  function repaint() {
    if (values.length < 2) {
      path.setAttribute("points", "");
      baselineEl.style.display = "none";
      return;
    }
    let min = Infinity;
    let max = -Infinity;
    for (const v of values) {
      if (v < min) min = v;
      if (v > max) max = v;
    }
    if (baseline != null) {
      if (baseline < min) min = baseline;
      if (baseline > max) max = baseline;
    }
    // Pad so a flat-line series doesn't collapse to height 0.
    if (max - min < 1e-6) {
      max = min + 1;
    }
    const span = max - min;

    const stepX = width / Math.max(1, values.length - 1);
    let pts = "";
    for (let i = 0; i < values.length; i++) {
      const x = i * stepX;
      const v = values[i] ?? min;
      const y = height - ((v - min) / span) * height;
      pts += `${x.toFixed(1)},${y.toFixed(2)} `;
    }
    path.setAttribute("points", pts.trim());

    if (baseline != null) {
      const y = height - ((baseline - min) / span) * height;
      baselineEl.setAttribute("y1", y.toFixed(2));
      baselineEl.setAttribute("y2", y.toFixed(2));
      baselineEl.style.display = "";
    } else {
      baselineEl.style.display = "none";
    }
  }

  return {
    el: wrap,
    setValues(v) {
      values = v;
      repaint();
    },
    setBaseline(v) {
      baseline = v;
      repaint();
    },
    setTone(t) {
      if (tone === t) return;
      tone = t;
      applyTone();
    },
  };
}
