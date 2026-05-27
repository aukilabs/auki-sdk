// `identityChip` — small accent-coloured pill used wherever the UI
// surfaces a degraded-identity / misconfigured-daemon signal:
//
//   - Robot card with no `/api/info` (stale SDK or unreachable).
//   - Past sessions with a manifest missing `app_id`.
//   - Generated-cluster failures listed below the cluster header.
//   - Park's own degradations from `/api/info.warnings`.
//
// We intentionally use the same pill across these surfaces so the
// operator learns one visual cue ("orange chip = something is not as
// it should be") instead of decoding per-surface vocabulary.
//
// Shape lifted from the existing `bg-accent/10` + `text-accent`
// pattern used elsewhere (e.g. error banners in `views/robot/structure.ts`),
// scaled down to fit inline next to row text.

import { escapeHtml } from "../util/escape";

export type IdentityChipOpts = {
  /** Headline text on the chip — kept short (one or two words). */
  label: string;
  /** Optional free-form detail surfaced via `title=` for hover. */
  detail?: string;
  /** Extra Tailwind classes appended to the base styling. */
  className?: string;
};

/** Build a chip element. Caller is responsible for appending it to
 * the DOM and disposing it (no internal state, so disposal is just
 * `el.remove()`). */
export function identityChip(opts: IdentityChipOpts): HTMLElement {
  const el = document.createElement("span");
  el.className = [
    "inline-flex items-center gap-1 px-1.5 py-0.5 rounded-sm",
    "border border-accent/60 bg-accent/10 text-accent",
    "text-[11px] uppercase tracking-[0.12em] whitespace-nowrap",
    opts.className ?? "",
  ]
    .filter(Boolean)
    .join(" ");
  el.style.fontFamily = "var(--font-display)";
  el.innerHTML = `<span class="w-1 h-1 rounded-full bg-accent shrink-0"></span><span>${escapeHtml(opts.label)}</span>`;
  if (opts.detail) el.title = opts.detail;
  return el;
}

/** Map a server-side failure `reason` (from `FailedPeer.reason` in
 * src/serve.rs or similar wire shapes) to a short operator-facing
 * label. Unknown reasons fall through to a generic "investigate" tag
 * — the `detail` field still carries the raw text. */
export function labelForFailureReason(reason: string): string {
  switch (reason) {
    case "unreachable":
      return "unreachable";
    case "bad_json":
      return "bad json";
    case "no_peer_id":
      return "no peer_id";
    case "no_app":
      return "no app";
    case "no_app_id":
      return "no app_id";
    default:
      return "investigate";
  }
}
