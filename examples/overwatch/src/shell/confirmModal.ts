// Generic yes/no confirmation modal. Returns a Promise<boolean>.
//
// Usage:
//   const ok = await confirm({
//     title: "Remove robot?",
//     message: "This will delete it from the cluster.",
//     confirmLabel: "Delete",
//     danger: true,
//   });
//   if (!ok) return;

import { escapeHtml } from "../util/escape";

export type ConfirmOpts = {
  title: string;
  message: string;
  confirmLabel?: string;
  cancelLabel?: string;
  /** When true, the confirm button uses red/destructive styling. */
  danger?: boolean;
};

export function confirm(opts: ConfirmOpts): Promise<boolean> {
  return new Promise((resolve) => {
    const overlay = document.createElement("div");
    overlay.className =
      "fixed inset-0 z-[60] flex items-center justify-center bg-ink/75 backdrop-blur-sm px-4";

    const panel = document.createElement("div");
    panel.className =
      "relative w-full max-w-sm rounded-md border border-paper/15 bg-ink-alt shadow-2xl p-5";
    overlay.appendChild(panel);

    const confirmLabel = opts.confirmLabel ?? "Confirm";
    const cancelLabel = opts.cancelLabel ?? "Cancel";
    const confirmClass = opts.danger
      ? "px-3 py-1.5 rounded-sm bg-red-500/20 hover:bg-red-500/30 border border-red-500/50 text-red-200 hover:text-red-100 text-[12px] transition-colors"
      : "px-3 py-1.5 rounded-sm bg-accent/20 hover:bg-accent/30 border border-accent/50 text-paper text-[12px] transition-colors";

    panel.innerHTML = `
      <div class="text-paper text-base font-medium mb-2" style="font-family: var(--font-display)">${escapeHtml(opts.title)}</div>
      <div class="text-rule/85 text-[13px] leading-relaxed mb-5">${escapeHtml(opts.message)}</div>
      <div class="flex items-center justify-end gap-2">
        <button class="px-3 py-1.5 rounded-sm border border-paper/15 hover:border-paper/30 text-rule hover:text-paper text-[12px] transition-colors" data-region="cancel">${escapeHtml(cancelLabel)}</button>
        <button class="${confirmClass}" data-region="confirm">${escapeHtml(confirmLabel)}</button>
      </div>
    `;

    const cleanup = () => {
      overlay.remove();
      document.removeEventListener("keydown", onKey);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") { cleanup(); resolve(false); }
      if (e.key === "Enter") { cleanup(); resolve(true); }
    };
    document.addEventListener("keydown", onKey);

    overlay.addEventListener("click", (e) => {
      if (e.target === overlay) { cleanup(); resolve(false); }
    });

    panel.querySelector('[data-region="cancel"]')!.addEventListener("click", () => {
      cleanup();
      resolve(false);
    });
    panel.querySelector('[data-region="confirm"]')!.addEventListener("click", () => {
      cleanup();
      resolve(true);
    });

    document.body.appendChild(overlay);
    (panel.querySelector('[data-region="confirm"]') as HTMLButtonElement).focus();
  });
}
