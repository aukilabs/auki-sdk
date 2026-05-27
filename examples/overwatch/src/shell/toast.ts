import { escapeHtml } from "../util/escape";
// Toast — bottom-stacked transient notifications. One global stack,
// auto-dismiss after a kind-dependent default. Kinds: info (rule),
// success (accent), error (accent + persistent until dismissed).
//
// Use:
//   toast.info("Snapshot saved", { actionLabel: "Reveal", onAction });
//   toast.success("Recording stopped · 12 MB");
//   toast.error("Record failed: HTTP 500");

type Kind = "info" | "success" | "error";

export type ToastOpts = {
  durationMs?: number;
  actionLabel?: string;
  onAction?: () => void;
  thumbnailUrl?: string;
};

let stackEl: HTMLElement | null = null;

function ensureStack(): HTMLElement {
  if (stackEl) return stackEl;
  stackEl = document.createElement("div");
  stackEl.className =
    "fixed bottom-4 left-1/2 -translate-x-1/2 z-[60] flex flex-col gap-2 items-center pointer-events-none";
  document.body.appendChild(stackEl);
  return stackEl;
}

function show(kind: Kind, message: string, opts: ToastOpts = {}): void {
  const stack = ensureStack();
  const el = document.createElement("div");
  const base =
    "pointer-events-auto flex items-center gap-3 max-w-[480px] px-3 py-2 rounded-md border backdrop-blur-sm shadow-lg text-xs transition-all duration-200";
  const tone =
    kind === "error"
      ? "bg-ink-alt/95 border-accent text-paper"
      : kind === "success"
        ? "bg-ink-alt/95 border-accent/60 text-paper"
        : "bg-ink-alt/95 border-paper/15 text-paper";
  el.className = `${base} ${tone} opacity-0 translate-y-1`;

  const dot =
    kind === "error"
      ? `<span class="w-2 h-2 rounded-sm bg-accent animate-pulse shrink-0"></span>`
      : kind === "success"
        ? `<span class="w-2 h-2 rounded-full bg-accent shrink-0"></span>`
        : `<span class="w-2 h-2 rounded-full bg-paper/40 shrink-0"></span>`;

  const thumb = opts.thumbnailUrl
    ? `<img src="${opts.thumbnailUrl}" class="w-8 h-8 object-cover rounded-sm shrink-0 border border-paper/15" alt="" />`
    : "";

  el.innerHTML = `
    ${thumb || dot}
    <span class="flex-1 min-w-0 leading-snug">${escapeHtml(message)}</span>
  `;

  if (opts.actionLabel && opts.onAction) {
    const btn = document.createElement("button");
    btn.className =
      "shrink-0 text-[12px] uppercase tracking-[0.1em] text-accent hover:text-paper px-2 py-1 transition-colors";
    btn.textContent = opts.actionLabel;
    btn.addEventListener("click", () => {
      try {
        opts.onAction!();
      } finally {
        dismiss();
      }
    });
    el.appendChild(btn);
  }

  const closeBtn = document.createElement("button");
  closeBtn.className =
    "shrink-0 w-5 h-5 flex items-center justify-center text-rule hover:text-paper transition-colors";
  closeBtn.textContent = "×";
  closeBtn.title = "Dismiss";
  closeBtn.addEventListener("click", () => dismiss());
  el.appendChild(closeBtn);

  stack.appendChild(el);
  // Animate in.
  requestAnimationFrame(() => {
    el.classList.remove("opacity-0", "translate-y-1");
  });

  let dismissed = false;
  const dismiss = () => {
    if (dismissed) return;
    dismissed = true;
    el.classList.add("opacity-0", "translate-y-1");
    setTimeout(() => el.remove(), 220);
  };

  const dur = opts.durationMs ?? (kind === "error" ? 0 : 3500);
  if (dur > 0) {
    setTimeout(dismiss, dur);
  }
}

export const toast = {
  info: (message: string, opts?: ToastOpts) => show("info", message, opts),
  success: (message: string, opts?: ToastOpts) => show("success", message, opts),
  error: (message: string, opts?: ToastOpts) => show("error", message, opts),
};

