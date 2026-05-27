// Park self-warnings banner — surfaces `info.warnings` from
// `GET /api/info` (Park's own response) as a row of chips at the top
// of any view that wants to call out "Park itself isn't fully healthy".
//
// Today the only warnings come from `cluster::init`:
//   - `app_instance::derive` failure (no MAC enumeration → empty
//     `app_instance` on the wire, no real `session_clock_id`).
//   - Clock-registry write failure (couldn't persist the session
//     clock entry — peers can't resolve our session clock until that's
//     fixed).
//
// Previously these only appeared as `eprintln!` in Park's terminal.
// That's invisible to a UI operator, which is exactly the
// silently-degraded behaviour we removed; this banner is the visible
// counterpart.

import { identityChip } from "./identityChip";

export type ParkSelfWarningsHandle = {
  el: HTMLElement;
  /** Replace the rendered warnings. Pass an empty array to hide the
   * banner entirely (the element stays in the DOM but collapses). */
  setWarnings(warnings: string[]): void;
};

export function makeParkSelfWarnings(): ParkSelfWarningsHandle {
  const el = document.createElement("div");
  el.className = "hidden";

  const setWarnings = (warnings: string[]): void => {
    el.replaceChildren();
    if (warnings.length === 0) {
      el.className = "hidden";
      return;
    }
    el.className =
      "mb-4 rounded-md border border-accent/40 bg-accent/5 px-4 py-3";
    const header = document.createElement("div");
    header.className =
      "flex items-center gap-2 text-paper/85 text-[12px] mb-2";
    header.style.fontFamily = "var(--font-display)";
    header.innerHTML = `
      <span class="w-1.5 h-1.5 rounded-full bg-accent"></span>
      <span>Park reports ${warnings.length} self-degradation${warnings.length === 1 ? "" : "s"}</span>
    `;
    el.appendChild(header);
    const list = document.createElement("div");
    list.className = "flex flex-col gap-1.5";
    for (const w of warnings) {
      const row = document.createElement("div");
      row.className = "flex items-center gap-2 text-[12px] text-paper/80";
      row.appendChild(
        identityChip({
          label: shortLabelForWarning(w),
          detail: w,
        }),
      );
      const detail = document.createElement("span");
      detail.className = "min-w-0 truncate";
      detail.title = w;
      detail.textContent = w;
      row.appendChild(detail);
      list.appendChild(row);
    }
    el.appendChild(list);
  };

  return { el, setWarnings };
}

/** Map a free-form warning string into a short chip label. We
 * pattern-match on the prefixes Park's `cluster::init` produces; any
 * unrecognised warning falls back to a generic "warning" chip with the
 * full text on hover. */
function shortLabelForWarning(w: string): string {
  if (w.startsWith("app_instance::derive failed")) return "no app_instance";
  if (w.startsWith("could not write session clock entry"))
    return "no clock entry";
  if (w.startsWith("session clock not registered")) return "no clock entry";
  return "warning";
}
