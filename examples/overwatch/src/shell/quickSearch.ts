// Quick search — global Spotlight-/Finder-style overlay. Mounted once
// at app boot, lives outside the route-driven #app tree so route
// changes don't unmount it.

import { escapeHtml } from "../util/escape";
import type { Daemon } from "../data/daemons";
import { subscribeDaemons } from "../data/daemons";
import { navigate, onRouteChange } from "../data/router";
import { iconRobot, iconSearch, iconSettings } from "../icons";
import { showSettingsOverlay } from "./settingsOverlay";
import { fadeUpIn, fadeDownOut, fade } from "../anim";

type Action = {
  id: "settings";
  label: string;
  sub?: string;
};

type Result =
  | { kind: "robot-live"; d: Daemon; key: string }
  | { kind: "action"; a: Action; key: string };

const ACTIONS: Action[] = [
  { id: "settings", label: "Open settings" },
];

export type QuickSearchHandle = {
  open(): void;
  close(): void;
  toggle(): void;
  dispose(): void;
};

export function mountQuickSearch(): QuickSearchHandle {
  // ─── DOM ──────────────────────────────────────────────────────────────
  const root = document.createElement("div");
  root.className =
    "fixed inset-0 bg-ink/70 backdrop-blur-sm z-50 hidden flex items-start justify-center";
  root.innerHTML = `
    <div class="w-full max-w-[640px] mx-4 mt-[18vh] bg-ink-alt border border-paper/10 rounded-md shadow-2xl overflow-hidden flex flex-col" data-region="panel">
      <div class="flex items-center gap-3 px-4 py-3 border-b border-paper/10 text-rule">
        <span data-region="search-icon">${iconSearch(18)}</span>
        <input
          type="text"
          autocomplete="off"
          spellcheck="false"
          placeholder="Search robots, actions…"
          class="flex-1 bg-transparent text-paper text-sm outline-none placeholder:text-rule/50"
          data-region="input"
        />
      </div>
      <div class="overflow-y-auto max-h-[60vh]" data-region="results"></div>
      <div class="px-4 py-2 border-t border-paper/10 text-rule/50 text-[11px] flex items-center gap-3">
        <span><kbd class="px-1 py-0.5 border border-paper/10 rounded text-[10px] mr-1 font-mono">↑↓</kbd>navigate</span>
        <span><kbd class="px-1 py-0.5 border border-paper/10 rounded text-[10px] mr-1 font-mono">↵</kbd>open</span>
        <span><kbd class="px-1 py-0.5 border border-paper/10 rounded text-[10px] mr-1 font-mono">esc</kbd>close</span>
      </div>
    </div>
  `;
  document.body.appendChild(root);

  const panel = root.querySelector('[data-region="panel"]') as HTMLElement;
  const input = root.querySelector('[data-region="input"]') as HTMLInputElement;
  const resultsEl = root.querySelector('[data-region="results"]') as HTMLElement;

  // ─── state ────────────────────────────────────────────────────────────
  let isOpen = false;
  let liveDaemons: Daemon[] = [];
  let query = "";
  let selectedIndex = 0;
  let currentResults: Result[] = [];

  const open = () => {
    if (isOpen) return;
    isOpen = true;
    root.classList.remove("hidden");
    input.value = "";
    query = "";
    selectedIndex = 0;
    repaint();
    // Animate backdrop fade + panel slide-up.
    root.style.opacity = "0";
    panel.style.opacity = "0";
    fade(root, 1);
    fadeUpIn(panel);
    setTimeout(() => input.focus(), 0);
  };

  const close = () => {
    if (!isOpen) return;
    isOpen = false;
    input.blur();
    // Run fades in parallel; hide once the slower one settles.
    fade(root, 0);
    fadeDownOut(panel).finished.then(() => {
      if (!isOpen) root.classList.add("hidden");
    });
  };

  const toggle = () => (isOpen ? close() : open());

  // ─── result computation ───────────────────────────────────────────────
  const buildResults = (): Result[] => {
    const q = query.trim().toLowerCase();

    const robotResults: Result[] = liveDaemons.map((d) => ({
      kind: "robot-live" as const,
      d,
      key: `robot-live:${d.url}`,
    }));

    const actionResults: Result[] = ACTIONS.map((a) => ({
      kind: "action" as const,
      a,
      key: `action:${a.id}`,
    }));

    if (!q) return [...robotResults, ...actionResults];

    const matches = (haystack: string) =>
      haystack.toLowerCase().includes(q);

    return [...robotResults, ...actionResults].filter((r) => {
      if (r.kind === "robot-live") {
        return (
          matches(r.d.name) ||
          matches(r.d.app) ||
          matches(r.d.url)
        );
      }
      return matches(r.a.label) || (r.a.sub != null && matches(r.a.sub));
    });
  };

  // ─── render ───────────────────────────────────────────────────────────
  const repaint = () => {
    currentResults = buildResults();
    if (selectedIndex >= currentResults.length) {
      selectedIndex = Math.max(0, currentResults.length - 1);
    }

    if (currentResults.length === 0) {
      const q = query.trim();
      const heading = q
        ? `No matches for &ldquo;${escapeHtml(q)}&rdquo;`
        : "Nothing to search yet";
      const sub = q
        ? `Try a different query, or press <kbd class="px-1 py-0.5 border border-paper/10 rounded text-[11px] font-mono">esc</kbd> to close.`
        : `Connect a device or wait for one to be discovered.`;
      resultsEl.innerHTML = `
        <div class="px-6 py-10 text-center">
          <p class="text-paper/80 text-xs mb-1">${heading}</p>
          <p class="text-rule/60 text-[12px]">${sub}</p>
        </div>
      `;
      return;
    }

    const groups = groupResults(currentResults);
    let renderedIdx = 0;
    const html: string[] = [];
    for (const [groupName, items] of groups) {
      html.push(
        `<div class="px-4 pt-3 pb-1 text-rule/70 text-[11px] uppercase tracking-[0.2em]" style="font-family: var(--font-display)">${escapeHtml(groupName)}</div>`,
      );
      for (const r of items) {
        html.push(rowHtml(r, renderedIdx === selectedIndex));
        renderedIdx++;
      }
    }
    resultsEl.innerHTML = html.join("");

    // Wire row clicks.
    const rows = resultsEl.querySelectorAll<HTMLElement>("[data-result-key]");
    rows.forEach((row, i) => {
      row.addEventListener("click", () => activate(i));
      row.addEventListener("mouseenter", () => {
        selectedIndex = i;
        paintSelection();
      });
    });

    // Scroll selected into view.
    const selected = resultsEl.querySelector<HTMLElement>("[data-selected='true']");
    if (selected) selected.scrollIntoView({ block: "nearest" });
  };

  const paintSelection = () => {
    const rows = resultsEl.querySelectorAll<HTMLElement>("[data-result-key]");
    rows.forEach((row, i) => {
      const sel = i === selectedIndex;
      row.dataset.selected = sel ? "true" : "false";
      // Re-apply class to reflect selection state.
      row.className = rowClass(sel);
    });
    const selected = resultsEl.querySelector<HTMLElement>("[data-selected='true']");
    if (selected) selected.scrollIntoView({ block: "nearest" });
  };

  const activate = (idx: number) => {
    const r = currentResults[idx];
    if (!r) return;
    if (r.kind === "robot-live") {
      navigate({ view: "robot", url: r.d.url });
      close();
    } else if (r.kind === "action") {
      close();
      if (r.a.id === "settings") showSettingsOverlay();
    }
  };

  // ─── data subs ────────────────────────────────────────────────────────
  const disposeDaemons = subscribeDaemons((next) => {
    liveDaemons = next;
    if (isOpen) repaint();
  });

  const disposeRoute = onRouteChange(() => {
    // Closing on route change keeps the overlay from leaking across
    // navigations after the user activates a result.
    close();
  });

  // ─── keyboard ─────────────────────────────────────────────────────────
  const onKeyDown = (e: KeyboardEvent) => {
    // Global open/close.
    const isMod = e.metaKey || e.ctrlKey;
    if (isMod && e.key.toLowerCase() === "k") {
      e.preventDefault();
      toggle();
      return;
    }
    if (e.key === "/" && !isOpen) {
      // Only when not already typing in an input/textarea.
      const tgt = e.target as HTMLElement | null;
      if (tgt && (tgt.tagName === "INPUT" || tgt.tagName === "TEXTAREA")) return;
      e.preventDefault();
      open();
      return;
    }

    if (!isOpen) return;

    if (e.key === "Escape") {
      e.preventDefault();
      close();
      return;
    }
    if (e.key === "ArrowDown") {
      e.preventDefault();
      selectedIndex = Math.min(currentResults.length - 1, selectedIndex + 1);
      paintSelection();
      return;
    }
    if (e.key === "ArrowUp") {
      e.preventDefault();
      selectedIndex = Math.max(0, selectedIndex - 1);
      paintSelection();
      return;
    }
    if (e.key === "Enter") {
      e.preventDefault();
      activate(selectedIndex);
      return;
    }
  };
  window.addEventListener("keydown", onKeyDown);

  // Click on backdrop closes; click on the panel itself doesn't.
  root.addEventListener("click", (e) => {
    if (e.target === root) close();
  });
  panel.addEventListener("click", (e) => e.stopPropagation());

  // Input → query.
  input.addEventListener("input", () => {
    query = input.value;
    selectedIndex = 0;
    repaint();
  });

  return {
    open,
    close,
    toggle,
    dispose() {
      disposeDaemons();
      disposeRoute();
      window.removeEventListener("keydown", onKeyDown);
      root.remove();
    },
  };
}

// ─── result helpers ────────────────────────────────────────────────────────

function groupResults(results: Result[]): Array<[string, Result[]]> {
  const robots = results.filter((r) => r.kind === "robot-live");
  const actions = results.filter((r) => r.kind === "action");
  const groups: Array<[string, Result[]]> = [];
  if (robots.length > 0) groups.push(["Devices", robots]);
  if (actions.length > 0) groups.push(["Actions", actions]);
  return groups;
}

function rowHtml(r: Result, selected: boolean): string {
  return `<button data-result-key="${escapeHtml(r.key)}" data-selected="${selected}" class="${rowClass(selected)}">${rowInner(r)}</button>`;
}

function rowClass(selected: boolean): string {
  const base =
    "w-full text-left px-4 py-2.5 flex items-center gap-3 transition-colors border-l-2";
  return selected
    ? `${base} bg-ink/60 border-accent text-paper`
    : `${base} border-transparent text-paper/85 hover:bg-ink/40`;
}

function rowInner(r: Result): string {
  if (r.kind === "robot-live") {
    return `
      <span class="text-paper/70 shrink-0">${iconRobot(20)}</span>
      <div class="flex-1 min-w-0">
        <div class="text-sm text-paper truncate">${escapeHtml(r.d.name)}</div>
        <div class="text-[12px] text-rule truncate">${escapeHtml(r.d.app)} · ${escapeHtml(r.d.url)}</div>
      </div>
      <span class="flex items-center gap-1.5 text-[11px] text-rule shrink-0">
        <span class="w-1.5 h-1.5 rounded-full bg-accent animate-pulse"></span>
        live
      </span>
    `;
  }
  return `
    <span class="text-paper/70 shrink-0">${iconSettings(20)}</span>
    <div class="flex-1 min-w-0">
      <div class="text-sm text-paper truncate">${escapeHtml(r.a.label)}</div>
      ${r.a.sub != null ? `<div class="text-[12px] text-rule truncate">${escapeHtml(r.a.sub)}</div>` : ""}
    </div>
  `;
}
