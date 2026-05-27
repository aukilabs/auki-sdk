// Inspector drawer — right-side slide-in panel surfacing the full
// metadata for whatever the operator is inspecting. Currently used
// from live video tiles ("frame inspector"); designed generic so any
// tile or surface can populate it.
//
// Singleton: opening a new content payload replaces the previous.
// Dismiss: × button, Esc, or click outside. Live consumers can call
// update() on every frame; the drawer batches via requestAnimationFrame
// so streaming updates don't thrash layout.
//
// Click-to-copy on every value: each row picks up a tiny copy hint on
// hover and writes the row's `copy` (or `value`) to the clipboard on
// click, with a brief "copied" toast so the action is acknowledged.

import { escapeHtml } from "../util/escape";
import { iconClose } from "../icons";
import { toast } from "./toast";

export type InspectorRow = {
  key: string;
  /** Plain string OR pre-formatted HTML. Use `html` when you need
   * mixed styling within the value (e.g. truncated peer ID with a
   * full-value title attribute). */
  value: string;
  html?: boolean;
  /** Render in monospace tabular-nums. Default true for IDs / numbers,
   * false for prose. */
  mono?: boolean;
  /** Override what gets copied. Defaults to `value` (raw text only). */
  copy?: string;
  /** Optional dim treatment for "n/a" / placeholder rows. */
  dim?: boolean;
};

export type InspectorSection = {
  title: string;
  rows: InspectorRow[];
};

export type InspectorAction = {
  id: string;
  label: string;
  icon?: string;
  title?: string;
  tone?: "default" | "accent" | "danger";
  disabled?: boolean;
  onClick: () => void;
};

export type InspectorContent = {
  /** Top-line label, e.g. `K1-AABB/head_left_cam`. */
  title: string;
  /** Optional secondary line below the title (e.g. daemon name). */
  subtitle?: string;
  /** Optional badge shown next to the title. */
  badge?: { label: string; tone?: "live" | "muted" | "warn" };
  /** Optional action buttons rendered below the header. */
  actions?: InspectorAction[];
  sections: InspectorSection[];
};

export type InspectorHandle = {
  /** Replace the panel's content. Cheap to call on every frame. */
  update(content: InspectorContent): void;
  close(): void;
  isOpen(): boolean;
};

let active: {
  /** Monotonic owner token. Each `openInspector` mints a new one; older
   * handles' update/close calls become no-ops once a newer caller takes
   * ownership. Without this, a tile that opened the drawer earlier
   * would keep pushing frames into the panel after another tile
   * replaced its content — silent corruption. */
  ownerToken: number;
  rootEl: HTMLElement;
  panelEl: HTMLElement;
  bodyEl: HTMLElement;
  actionBarEl: HTMLElement;
  titleEl: HTMLElement;
  subtitleEl: HTMLElement;
  badgeEl: HTMLElement;
  pendingContent: InspectorContent | null;
  rafHandle: number | null;
  onKey: (e: KeyboardEvent) => void;
  onOutside: (e: Event) => void;
} | null = null;

let nextOwnerToken = 1;

export function openInspector(initial: InspectorContent): InspectorHandle {
  if (active) {
    active.ownerToken = ++nextOwnerToken;
    queueRender(initial);
    return makeHandle(active.ownerToken);
  }

  const root = document.createElement("div");
  root.className =
    "fixed inset-0 z-40 pointer-events-none";

  const scrim = document.createElement("div");
  scrim.className =
    "absolute inset-0 bg-ink/30 opacity-0 transition-opacity duration-200 pointer-events-auto";
  root.appendChild(scrim);

  const panel = document.createElement("aside");
  panel.className =
    "absolute top-14 bottom-0 right-0 w-[360px] max-w-[90vw] bg-ink-alt border-l border-paper/10 shadow-2xl translate-x-full transition-transform duration-200 ease-out flex flex-col pointer-events-auto";
  panel.setAttribute("role", "complementary");
  panel.setAttribute("aria-label", "Inspector");
  root.appendChild(panel);

  const header = document.createElement("div");
  header.className =
    "flex items-start justify-between gap-3 px-4 pt-3.5 pb-3 border-b border-paper/10 shrink-0";
  panel.appendChild(header);

  const headLeft = document.createElement("div");
  headLeft.className = "flex flex-col gap-0.5 min-w-0";
  header.appendChild(headLeft);

  const titleRow = document.createElement("div");
  titleRow.className = "flex items-center gap-2 min-w-0";
  headLeft.appendChild(titleRow);

  const title = document.createElement("div");
  title.className =
    "text-paper text-sm font-medium truncate";
  titleRow.appendChild(title);

  const badge = document.createElement("span");
  badge.className = "hidden";
  titleRow.appendChild(badge);

  const subtitle = document.createElement("div");
  subtitle.className = "text-rule text-[12px] truncate";
  headLeft.appendChild(subtitle);

  const closeBtn = document.createElement("button");
  closeBtn.className =
    "shrink-0 w-7 h-7 -mr-1 flex items-center justify-center text-rule hover:text-paper rounded-sm transition-colors";
  closeBtn.title = "Close (Esc)";
  closeBtn.innerHTML = iconClose(14);
  header.appendChild(closeBtn);

  const actionBar = document.createElement("div");
  actionBar.className =
    "hidden px-4 py-2 border-b border-paper/10 shrink-0 flex items-center gap-2";
  panel.appendChild(actionBar);

  const body = document.createElement("div");
  body.className =
    "flex-1 min-h-0 overflow-y-auto px-4 py-3 text-xs space-y-4";
  panel.appendChild(body);

  document.body.appendChild(root);

  // Animate in.
  requestAnimationFrame(() => {
    scrim.classList.remove("opacity-0");
    scrim.classList.add("opacity-100");
    panel.classList.remove("translate-x-full");
  });

  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Escape") closeInternal();
  };
  const onOutside = (e: Event) => {
    if (!(e.target instanceof Node)) return;
    if (e.target === scrim) closeInternal();
  };

  document.addEventListener("keydown", onKey);
  scrim.addEventListener("click", onOutside);
  closeBtn.addEventListener("click", () => closeInternal());

  const token = ++nextOwnerToken;
  active = {
    ownerToken: token,
    rootEl: root,
    panelEl: panel,
    bodyEl: body,
    actionBarEl: actionBar,
    titleEl: title,
    subtitleEl: subtitle,
    badgeEl: badge,
    pendingContent: null,
    rafHandle: null,
    onKey,
    onOutside,
  };

  queueRender(initial);
  return makeHandle(token);
}

function makeHandle(token: number): InspectorHandle {
  return {
    update(content) {
      if (!active || active.ownerToken !== token) return;
      queueRender(content);
    },
    close() {
      if (!active || active.ownerToken !== token) return;
      closeInternal();
    },
    isOpen() {
      return active != null && active.ownerToken === token;
    },
  };
}

function queueRender(content: InspectorContent) {
  if (!active) return;
  active.pendingContent = content;
  if (active.rafHandle != null) return;
  active.rafHandle = requestAnimationFrame(() => {
    if (!active) return;
    active.rafHandle = null;
    if (active.pendingContent) {
      paint(active.pendingContent);
      active.pendingContent = null;
    }
  });
}

function paint(content: InspectorContent) {
  if (!active) return;
  active.titleEl.textContent = content.title;
  active.subtitleEl.textContent = content.subtitle ?? "";
  active.subtitleEl.style.display = content.subtitle ? "" : "none";

  if (content.badge) {
    const tone = content.badge.tone ?? "muted";
    const palette: Record<string, string> = {
      live: "border-accent/40 text-accent",
      muted: "border-paper/15 text-rule",
      warn: "border-yellow-400/40 text-yellow-300",
    };
    active.badgeEl.className = `text-[10px] uppercase tracking-[0.12em] px-1.5 py-0.5 rounded-sm border shrink-0 ${palette[tone]}`;
    active.badgeEl.textContent = content.badge.label;
    active.badgeEl.classList.remove("hidden");
  } else {
    active.badgeEl.classList.add("hidden");
  }

  paintActions(content.actions ?? []);

  // Re-render body. Cheap: a few sections, a few rows each.
  active.bodyEl.replaceChildren();
  for (const section of content.sections) {
    active.bodyEl.appendChild(renderSection(section));
  }
}

function paintActions(actions: InspectorAction[]) {
  if (!active) return;
  active.actionBarEl.replaceChildren();
  if (actions.length === 0) {
    active.actionBarEl.classList.add("hidden");
    return;
  }
  active.actionBarEl.classList.remove("hidden");
  for (const action of actions) {
    const b = document.createElement("button");
    const tone = action.tone ?? "default";
    const palette: Record<string, string> = {
      default: "border-paper/15 text-paper/85 hover:border-paper/30 hover:bg-paper/5",
      accent: "border-accent/50 bg-accent/15 text-accent hover:bg-accent/20",
      danger: "border-red-400/45 bg-red-500/10 text-red-300 hover:bg-red-500/15",
    };
    b.className =
      `inline-flex items-center gap-1.5 rounded-sm border px-2.5 py-1.5 text-[11px] ` +
      `uppercase tracking-[0.12em] transition-colors ${palette[tone]}`;
    b.disabled = action.disabled ?? false;
    b.title = action.title ?? action.label;
    b.innerHTML = `${action.icon ? `<span>${action.icon}</span>` : ""}<span>${escapeHtml(action.label)}</span>`;
    if (b.disabled) {
      b.classList.add("opacity-50", "cursor-not-allowed");
    } else {
      b.addEventListener("click", (e) => {
        e.stopPropagation();
        action.onClick();
      });
    }
    active.actionBarEl.appendChild(b);
  }
}

function renderSection(s: InspectorSection): HTMLElement {
  const wrap = document.createElement("section");
  const heading = document.createElement("h3");
  heading.className =
    "text-rule/70 uppercase tracking-[0.15em] text-[10px] font-medium mb-1.5";
  heading.textContent = s.title;
  wrap.appendChild(heading);

  const dl = document.createElement("dl");
  dl.className = "space-y-1";
  for (const row of s.rows) {
    dl.appendChild(renderRow(row));
  }
  wrap.appendChild(dl);
  return wrap;
}

function renderRow(row: InspectorRow): HTMLElement {
  const wrap = document.createElement("div");
  wrap.className =
    "group flex items-baseline justify-between gap-3 cursor-pointer rounded-sm px-1 -mx-1 hover:bg-paper/5 transition-colors";

  const dt = document.createElement("dt");
  dt.className = "text-rule/70 text-[11px] shrink-0 uppercase tracking-[0.08em]";
  dt.textContent = row.key;
  wrap.appendChild(dt);

  const ddWrap = document.createElement("span");
  ddWrap.className = "flex items-center gap-1.5 min-w-0 max-w-[60%]";
  const dd = document.createElement("dd");
  const baseClasses = row.mono === false ? "" : "font-mono tabular-nums";
  const dim = row.dim ? "text-rule/60" : "text-paper/85";
  dd.className = `truncate text-[12px] ${baseClasses} ${dim}`;
  if (row.html) {
    dd.innerHTML = row.value;
  } else {
    dd.textContent = row.value;
  }
  ddWrap.appendChild(dd);

  const copyHint = document.createElement("span");
  copyHint.className =
    "text-rule/60 group-hover:text-paper/80 text-[10px] uppercase tracking-[0.08em] opacity-0 group-hover:opacity-100 transition-opacity shrink-0";
  copyHint.textContent = "copy";
  ddWrap.appendChild(copyHint);
  wrap.appendChild(ddWrap);

  wrap.addEventListener("click", () => {
    const text = row.copy ?? (row.html ? stripHtml(row.value) : row.value);
    void copyToClipboard(text, row.key);
  });

  return wrap;
}

async function copyToClipboard(text: string, key: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(text);
    toast.info(`Copied ${escapeHtml(key)}`);
  } catch {
    toast.error("Couldn't copy to clipboard");
  }
}

function stripHtml(s: string): string {
  const div = document.createElement("div");
  div.innerHTML = s;
  return div.textContent ?? "";
}

function closeInternal() {
  if (!active) return;
  const a = active;
  active = null;
  a.panelEl.classList.add("translate-x-full");
  const scrim = a.rootEl.querySelector("div") as HTMLElement | null;
  if (scrim) {
    scrim.classList.remove("opacity-100");
    scrim.classList.add("opacity-0");
  }
  document.removeEventListener("keydown", a.onKey);
  if (a.rafHandle != null) cancelAnimationFrame(a.rafHandle);
  setTimeout(() => a.rootEl.remove(), 220);
}

/** Whether an inspector is currently mounted. Used by tile-level
 * keyboard handlers (e.g. `i` shortcut) to decide whether to open. */
export function isInspectorOpen(): boolean {
  return active != null;
}

/** Imperative close — used when navigating away from a route so the
 * drawer doesn't linger over the next view. */
export function closeInspector(): void {
  closeInternal();
}
