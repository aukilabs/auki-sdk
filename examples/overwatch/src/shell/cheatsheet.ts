// Cheatsheet — keyboard shortcuts overlay. Triggered by `?`. Shows
// the shortcuts available on the current view; the caller passes a
// list of {key, description} entries.

import { escapeHtml } from "../util/escape";
import { fade, fadeDownOut, fadeUpIn } from "../anim";

export type Shortcut = {
  key: string;
  description: string;
  group?: string;
};

let openCheatsheet: HTMLElement | null = null;

export function showCheatsheet(shortcuts: Shortcut[]): void {
  if (openCheatsheet) {
    closeCheatsheet();
    return;
  }

  const root = document.createElement("div");
  root.className =
    "fixed inset-0 bg-ink/70 backdrop-blur-sm z-50 flex items-center justify-center";

  const grouped = new Map<string, Shortcut[]>();
  for (const s of shortcuts) {
    const g = s.group ?? "Actions";
    if (!grouped.has(g)) grouped.set(g, []);
    grouped.get(g)!.push(s);
  }

  const groupHtml = Array.from(grouped.entries())
    .map(
      ([groupName, items]) => `
    <div class="mb-4 last:mb-0">
      <div class="text-rule text-[11px] uppercase tracking-[0.2em] mb-2" style="font-family: var(--font-display)">${escapeHtml(groupName)}</div>
      <dl class="space-y-1.5">
        ${items
          .map(
            (s) => `
          <div class="flex items-center justify-between gap-4 text-xs">
            <dt class="text-paper/85">${escapeHtml(s.description)}</dt>
            <dd>
              <kbd class="px-2 py-0.5 bg-ink border border-paper/15 rounded text-[12px] font-mono text-paper/90">${escapeHtml(s.key)}</kbd>
            </dd>
          </div>
        `,
          )
          .join("")}
      </dl>
    </div>
  `,
    )
    .join("");

  root.innerHTML = `
    <div class="w-full max-w-[460px] mx-4 bg-ink-alt border border-paper/10 rounded-md shadow-2xl overflow-hidden flex flex-col" data-region="panel">
      <div class="px-5 pt-4 pb-3 border-b border-paper/10 flex items-center justify-between">
        <h2 class="text-paper text-base font-medium" style="font-family: var(--font-display)">Keyboard shortcuts</h2>
        <button class="text-rule hover:text-paper text-sm" data-region="close" title="Close (Esc or ?)">×</button>
      </div>
      <div class="px-5 py-4 overflow-y-auto max-h-[60vh]">
        ${groupHtml}
      </div>
      <div class="px-5 py-2 border-t border-paper/10 text-[11px] text-rule/60 text-right">
        ? to toggle · Esc to close
      </div>
    </div>
  `;

  document.body.appendChild(root);
  openCheatsheet = root;

  const panel = root.querySelector('[data-region="panel"]') as HTMLElement;
  const closeBtn = root.querySelector('[data-region="close"]') as HTMLButtonElement;
  fade(root, 1);
  fadeUpIn(panel);

  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Escape" || e.key === "?") {
      e.preventDefault();
      closeCheatsheet();
    }
  };
  window.addEventListener("keydown", onKey);

  closeBtn.addEventListener("click", () => closeCheatsheet());
  root.addEventListener("click", (e) => {
    if (e.target === root) closeCheatsheet();
  });

  function closeCheatsheet() {
    if (openCheatsheet !== root) return;
    openCheatsheet = null;
    window.removeEventListener("keydown", onKey);
    fade(root, 0);
    fadeDownOut(panel).finished.then(() => root.remove());
  }
}

