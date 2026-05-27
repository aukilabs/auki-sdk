// Stage — multi-tile area showing every toggled-on sensor for the
// active robot. Layout adapts to tile count (see STAGE.md §2). Record
// + global controls live in the controls bar below the stage; mode
// switching (live ↔ session) lives in the sidebar dropdown.
//
// Tracks a "focused tile" (last-clicked / last-keyboard-targeted) so
// keyboard shortcuts (F freeze, S snapshot) know which tile to act on.

import type { Daemon } from "../../data/daemons";
import { makeTile, type TileHandle, type TileSpec } from "./tile";

export type StageHandle = {
  el: HTMLElement;
  setTiles(specs: TileSpec[]): void;
  getTiles(): TileHandle[];
  getFocusedTile(): TileHandle | null;
  dispose(): void;
};

export function makeStage(
  daemon: Daemon | undefined,
  opts: { onCloseTile: (sensor_id: string) => void },
): StageHandle {
  const el = document.createElement("section");
  el.className =
    "col-start-2 row-start-1 bg-ink-alt overflow-hidden relative min-h-0 flex flex-col";

  const grid = document.createElement("div");
  grid.className = "flex-1 min-h-0 p-3";
  el.appendChild(grid);

  const emptyEl = document.createElement("div");
  emptyEl.className = "flex-1 flex items-center justify-center p-6";
  emptyEl.innerHTML = daemon
    ? `
      <div class="text-center max-w-sm px-8 py-10 border border-dashed border-paper/10 rounded-md">
        <div class="text-paper/30 mb-4 flex justify-center">
          <svg width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
            <rect x="3" y="3" width="18" height="18" rx="2"/>
            <path d="M3 9h18M3 15h18M9 3v18M15 3v18" opacity="0.4"/>
          </svg>
        </div>
        <p class="text-paper/85 text-sm mb-1.5" style="font-family: var(--font-display)">Stage is empty</p>
        <p class="text-rule/70 text-[12px] leading-snug">Toggle a sensor in the bar below to bring it on stage. Multiple sensors can run side-by-side.</p>
      </div>
    `
    : `
      <div class="text-center max-w-sm px-8 py-10 border border-dashed border-paper/10 rounded-md">
        <p class="text-paper/85 text-sm mb-1.5" style="font-family: var(--font-display)">No device selected</p>
        <p class="text-rule/70 text-[12px] leading-snug">Pick a device from the dashboard to inspect its sensors.</p>
      </div>
    `;

  const tiles = new Map<string, TileHandle>();
  let focusedKey: string | null = null;

  const renderEmpty = () => {
    grid.replaceChildren();
    grid.className = "flex-1 min-h-0 flex";
    grid.appendChild(emptyEl);
  };

  const renderTiles = () => {
    grid.replaceChildren();
    const count = tiles.size;
    grid.className = `flex-1 min-h-0 grid gap-3 p-3 ${gridForCount(count)}`;
    for (const [key, tile] of tiles.entries()) {
      grid.appendChild(tile.el);
      // Wire focus tracking: click anywhere on the tile makes it focused.
      tile.el.addEventListener(
        "mousedown",
        () => {
          focusedKey = key;
          paintFocusRing();
        },
        { capture: true },
      );
    }
    if (!focusedKey || !tiles.has(focusedKey)) {
      // Default focus to the first tile.
      focusedKey = tiles.keys().next().value ?? null;
    }
    paintFocusRing();
  };

  const paintFocusRing = () => {
    for (const [key, tile] of tiles.entries()) {
      const isFocused = key === focusedKey;
      tile.el.classList.toggle("ring-1", isFocused);
      tile.el.classList.toggle("ring-accent/50", isFocused);
    }
  };

  const refresh = () => {
    if (tiles.size === 0) renderEmpty();
    else renderTiles();
  };

  refresh();

  return {
    el,
    setTiles(specs: TileSpec[]) {
      const wantedIds = new Set(specs.map((s) => keyOf(s)));

      for (const [id, tile] of tiles) {
        if (!wantedIds.has(id)) {
          tile.dispose();
          tiles.delete(id);
        }
      }
      for (const spec of specs) {
        const id = keyOf(spec);
        if (tiles.has(id)) continue;
        const tile = makeTile(spec, {
          onClose: () => opts.onCloseTile(spec.sensor_id),
        });
        tiles.set(id, tile);
      }

      const ordered = specs.map((s) => tiles.get(keyOf(s))).filter(Boolean) as TileHandle[];
      tiles.clear();
      for (let i = 0; i < ordered.length; i++) {
        const t = ordered[i]!;
        const s = specs[i]!;
        tiles.set(keyOf(s), t);
      }
      refresh();
    },
    getTiles() {
      return Array.from(tiles.values());
    },
    getFocusedTile() {
      if (focusedKey && tiles.has(focusedKey)) return tiles.get(focusedKey)!;
      return tiles.values().next().value ?? null;
    },
    dispose() {
      for (const tile of tiles.values()) tile.dispose();
      tiles.clear();
    },
  };
}

function keyOf(spec: TileSpec): string {
  switch (spec.kind) {
    case "video":
      return `video:${spec.sensor_id}`;
    case "point_cloud":
      return `point_cloud:${spec.sensor_id}`;
    case "k1_pose":
      return `k1_pose:${spec.sensor_id}`;
    case "world":
      return "world";
  }
}

function gridForCount(n: number): string {
  if (n === 1) return "grid-cols-1 grid-rows-1";
  if (n === 2) return "grid-cols-2 grid-rows-1";
  if (n <= 4) return "grid-cols-2 grid-rows-2";
  return "[grid-template-columns:repeat(auto-fit,minmax(360px,1fr))]";
}
