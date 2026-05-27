// Shared chrome for stage tiles. Every tile (camera, pointcloud, pose,
// future composite) follows the same frame:
//
//   ┌───────────────────────────────────────────────────────────┐
//   │ [icon] sensor-name                              [×]       │  ← top: identity chip + close
//   │                                                            │
//   │                  (tile body — image / canvas / video)      │
//   │                                                            │
//   │ [info: sensor_id · …]                       [actions: …]  │  ← bottom: info + per-tile controls
//   └───────────────────────────────────────────────────────────┘
//
// Tile-specific pieces (live preview, point cloud, pose tree, MP4
// playback) live in their own modules and slot into the body. The
// chrome itself is universal so close buttons, identity chips, and
// bottom-bar layout don't drift between tile types.

import { escapeHtml } from "../../../util/escape";
import { iconClose, iconForSensorType, sensorTypeFromEntry, type SensorType } from "../../../icons";
import type { SensorEntry } from "../../../data/registry";
import { shortName } from "./names";

/** Build a tile shell. Returns the outer element + slots the tile
 * implementation can fill (body, identity chip extras, bottom info,
 * bottom actions). The shell handles the close button + identity icon
 * + sensor-name label. */
export type TileChrome = {
  el: HTMLElement;
  /** Container the tile fills with its primary content
   * (`<img>` / `<canvas>` / `<video>` / Three.js root). Positioned
   * absolute inset-0 by default; tile content sits above it. */
  body: HTMLElement;
  /** Top-left chip element. Already contains icon + sensor name; the
   * tile may append additional controls (e.g. a recording-source
   * dropdown) by appending children. */
  topLeft: HTMLElement;
  /** Top-right corner. Already has the close button; tiles may add
   * other top-right controls (e.g. a fullscreen toggle) by appending. */
  topRight: HTMLElement;
  /** Bottom bar — left side (info) and right side (actions). Tiles
   * fill these with their type-specific content. */
  bottomInfo: HTMLElement;
  bottomActions: HTMLElement;
  /** Imperative helpers that work on the chip — used by tiles that
   * temporarily re-style it (e.g. live tile's "Frozen" pill). */
  setChipState(opts: { label?: string; iconHtml?: string; tone?: "default" | "accent" }): void;
};

export type TileChromeOpts = {
  sensor_id: string;
  /** SDK registry entry for the sensor — drives the chip icon.
   * Optional because pose tiles (`/joint_encoders` and past-session
   * `/pose` rows) don't have a registry kind and instead pass `type`
   * explicitly below. Live video / pointcloud tiles always pass an
   * entry; the chip then resolves to the SDK-declared kind rather
   * than guessing from the sensor_id string. */
  entry?: SensorEntry;
  /** Explicit override for tile types not backed by a registry entry
   * (pose tiles). Wins over `entry` when set. */
  type?: SensorType;
  onClose: () => void;
};

export function makeTileChrome(opts: TileChromeOpts): TileChrome {
  const type = opts.type ?? sensorTypeFromEntry(opts.sensor_id, opts.entry);
  const name = shortName(opts.sensor_id);

  const el = document.createElement("div");
  el.className =
    "group relative bg-ink rounded-md border border-paper/10 overflow-hidden min-h-0 transition-opacity";
  el.tabIndex = 0;

  // Body — tile content (img / canvas / video / Three.js root) goes
  // here. Absolute-positioned inset-0 so chrome sits above naturally.
  const body = document.createElement("div");
  body.className = "absolute inset-0";
  body.dataset.region = "tile-body";
  el.appendChild(body);

  // ─── top row ──────────────────────────────────────────────────────
  // Top-left: identity chip (icon + name) + space for tile-specific
  // controls underneath (e.g. recording dropdown).
  const topLeft = document.createElement("div");
  topLeft.className =
    "absolute top-2 left-2 flex flex-col gap-1.5 items-start pointer-events-none";
  topLeft.dataset.region = "tile-top-left";
  el.appendChild(topLeft);

  const chip = document.createElement("div");
  chip.className =
    "flex items-center gap-1.5 text-paper/90 text-[12px] px-2 py-1 bg-ink/65 backdrop-blur-sm rounded-sm transition-colors";
  chip.innerHTML = `
    <span class="text-accent" data-region="chip-icon">${iconForSensorType(type, 14)}</span>
    <span data-region="chip-label">${escapeHtml(name)}</span>
  `;
  topLeft.appendChild(chip);

  // Top-right: close + space for additional controls (e.g. fullscreen).
  const topRight = document.createElement("div");
  topRight.className =
    "absolute top-2 right-2 flex items-center gap-1 pointer-events-auto";
  topRight.dataset.region = "tile-top-right";
  el.appendChild(topRight);

  const closeBtn = makeChromeBtn(iconClose(14), "Close");
  closeBtn.addEventListener("click", () => opts.onClose());
  topRight.appendChild(closeBtn);

  // ─── bottom bar ───────────────────────────────────────────────────
  const bottomBar = document.createElement("div");
  bottomBar.className =
    "absolute bottom-0 inset-x-0 flex items-center justify-between gap-2 px-2 py-1.5 bg-ink/65 backdrop-blur-sm border-t border-paper/10 text-[11px] text-paper/75";
  bottomBar.dataset.region = "tile-bottom-bar";
  el.appendChild(bottomBar);

  const bottomInfo = document.createElement("div");
  bottomInfo.className = "flex items-center gap-3 min-w-0 font-mono";
  bottomBar.appendChild(bottomInfo);

  const bottomActions = document.createElement("div");
  bottomActions.className = "flex items-center gap-1 shrink-0";
  bottomBar.appendChild(bottomActions);

  // ─── helpers ──────────────────────────────────────────────────────
  const chipIconEl = chip.querySelector('[data-region="chip-icon"]') as HTMLElement;
  const chipLabelEl = chip.querySelector('[data-region="chip-label"]') as HTMLElement;

  const setChipState: TileChrome["setChipState"] = ({ label, iconHtml, tone }) => {
    if (label !== undefined) chipLabelEl.textContent = label;
    if (iconHtml !== undefined) chipIconEl.innerHTML = iconHtml;
    if (tone !== undefined) {
      if (tone === "accent") {
        chip.className =
          "flex items-center gap-1.5 text-[12px] px-2 py-1 backdrop-blur-sm rounded-sm transition-colors bg-accent text-ink uppercase tracking-[0.15em] font-medium";
        chipIconEl.className = "text-ink";
      } else {
        chip.className =
          "flex items-center gap-1.5 text-paper/90 text-[12px] px-2 py-1 bg-ink/65 backdrop-blur-sm rounded-sm transition-colors";
        chipIconEl.className = "text-accent";
      }
    }
  };

  return {
    el,
    body,
    topLeft,
    topRight,
    bottomInfo,
    bottomActions,
    setChipState,
  };
}

/** Square 28-px button styled to sit on top of a tile (e.g. Close,
 * Freeze). The SVG is set via innerHTML; pass a sized icon string. */
export function makeChromeBtn(svg: string, title: string): HTMLButtonElement {
  const b = document.createElement("button");
  b.className =
    "w-7 h-7 flex items-center justify-center text-paper/80 hover:text-paper bg-ink/65 backdrop-blur-sm rounded-sm transition-colors pointer-events-auto";
  b.title = title;
  b.innerHTML = svg;
  return b;
}
