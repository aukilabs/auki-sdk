// Directory view — landing page. One section:
//   Online — Park plus robots currently in Park's cluster
//
// Daemons are sourced exclusively from the cluster's peer list per
// PARK-Q4 (no mDNS, no manual fallback, no offline/seen history). The
// cluster doc is the source of "who's here"; if a peer isn't in it,
// it doesn't exist in Park's view. The server also requires
// participant-info before surfacing a peer as a live robot. The
// mDNS-era "Offline" section (LocalStorage-remembered daemons not
// currently live) is gone.

import type { Daemon } from "../../data/daemons";
import { subscribeDaemons } from "../../data/daemons";
import { makeBanner } from "./banner";
import { makeLiveCard, type CardHandle } from "./robotCard";
import { makeParkSelfCard } from "./parkSelfCard";

export function directory(): { el: HTMLElement; dispose: () => void } {
  const el = document.createElement("main");
  el.className = "flex-1 overflow-y-auto flex flex-col";

  const banner = makeBanner();
  el.appendChild(banner.el);

  const section = document.createElement("div");
  section.className = "px-8 py-8 flex-1 flex flex-col gap-10";
  section.innerHTML = `
    <div>
      <div class="mb-5 flex items-center justify-between gap-4 flex-wrap">
        <h2 class="text-rule text-[11px] uppercase tracking-[0.2em]" style="font-family: var(--font-display)">Robots</h2>
      </div>

      <div data-region="online-section">
        <div class="flex items-center gap-2 mb-3">
          <span class="w-1.5 h-1.5 rounded-full bg-accent animate-pulse"></span>
          <span class="text-rule text-[11px] uppercase tracking-[0.15em]" style="font-family: var(--font-display)">Online</span>
          <span class="text-rule/60 text-[11px]" data-region="online-count"></span>
        </div>
        <div data-region="online-grid"></div>
      </div>
    </div>
  `;
  el.appendChild(section);

  const onlineCountEl = section.querySelector('[data-region="online-count"]') as HTMLElement;
  const onlineGrid = section.querySelector('[data-region="online-grid"]') as HTMLElement;

  // ─── state ─────────────────────────────────────────────────────────────
  let live: Daemon[] = [];
  const cards = new Map<string, CardHandle>();

  const GRID = "grid grid-cols-[repeat(auto-fill,minmax(260px,1fr))] gap-3";

  const repaint = () => {
    onlineCountEl.textContent = `${live.length + 1}`;

    // Tear down stale cards. Park's self-card is always present.
    const wantedKeys = new Set(["self:park", ...live.map((d) => `live:${d.url}`)]);
    for (const [k, c] of cards) {
      if (!wantedKeys.has(k)) {
        c.dispose();
        cards.delete(k);
      }
    }

    onlineGrid.replaceChildren();
    const parkCard = cardFor("self:park", () => makeParkSelfCard());
    const parkItem = { app: "park", el: parkCard };
    if (live.length === 0) {
      // Park's own row always renders — there's at least one peer in
      // the cluster (us). The empty-state hint sits below.
      const grid = document.createElement("div");
      grid.className = GRID;
      grid.appendChild(parkCard);
      onlineGrid.appendChild(grid);
      onlineGrid.appendChild(onlineEmptyState());
    } else {
      const items = [
        parkItem,
        ...live.map((d) => ({
          app: d.app,
          el: cardFor(`live:${d.url}`, () => makeLiveCard(d)),
        })),
      ];
      onlineGrid.appendChild(buildGroups(items));
    }
  };

  const cardFor = (key: string, make: () => CardHandle): HTMLElement => {
    let c = cards.get(key);
    if (!c) {
      c = make();
      cards.set(key, c);
    }
    return c.el;
  };

  // One responsive grid for every robot regardless of app. Wraps to a
  // new row when the viewport runs out of horizontal space. Each
  // card's subtitle already shows its `app` so the per-app subheaders
  // that used to live here were redundant (and forced single-card
  // apps to stretch to a full row of their own).
  const buildGroups = (items: { app: string; el: HTMLElement }[]): DocumentFragment => {
    const frag = document.createDocumentFragment();
    const grid = document.createElement("div");
    grid.className = GRID;
    items.forEach((i) => grid.appendChild(i.el));
    frag.appendChild(grid);
    return frag;
  };

  const disposeDaemons = subscribeDaemons((next) => {
    live = next;
    repaint();
  });

  return {
    el,
    dispose: () => {
      disposeDaemons();
      banner.dispose();
      cards.forEach((c) => c.dispose());
      cards.clear();
    },
  };
}

function onlineEmptyState(): HTMLElement {
  const el = document.createElement("div");
  el.className =
    "px-6 py-10 text-center border border-dashed border-paper/10 rounded-md";
  el.innerHTML = `
    <div class="flex items-center justify-center gap-2 mb-3 text-accent">
      <span class="w-2 h-2 rounded-full bg-accent animate-pulse"></span>
      <span class="text-xs uppercase tracking-[0.2em]" style="font-family: var(--font-display)">scanning</span>
    </div>
    <p class="text-paper/85 text-base mb-1.5" style="font-family: var(--font-display)">No robots in cluster</p>
    <p class="text-rule/65 text-[12px] leading-relaxed max-w-sm mx-auto">
      Cluster peers will appear here automatically once they join Park's cluster.
    </p>
  `;
  return el;
}
