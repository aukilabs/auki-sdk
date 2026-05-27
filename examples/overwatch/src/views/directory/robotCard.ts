// Robot card — one tile in the directory grid. Three variants:
// - live: live thumbnail + state subscription + orange "live" pill, navigable
// - unreachable: manually-added daemon whose /api/info is failing — grey
//   pill, grey border, NOT navigable (clicking would lead to a broken view)
// - offline: previously-seen daemon no longer in the live list, grey pill,
//   NOT navigable
//
// Status pill (top-right of thumbnail) is the single source of operator
// signal: it carries colour + label uniformly across all three variants
// so they read as siblings, not as different things.

import { escapeHtml } from "../../util/escape";
import type { Daemon } from "../../data/daemons";
import type { DaemonSensorLogs } from "../../data/sensorLogs";
import { subscribeSensorLogs } from "../../data/sensorLogs";
import {
  subscribeInfo,
  formatAge,
  type InfoSnapshot,
} from "../../data/info";
import { subscribeCluster } from "../../data/cluster";
import { subscribePreview } from "../../data/preview";
import { navigate } from "../../data/router";
import {
  fetchCatalog,
  sensorEntryFromCatalogEntry,
  type SensorEntry,
  type SensorCatalogEntry,
} from "../../data/registry";
import {
  getDashboardPreviewSensor,
  setDashboardPreviewSensor,
} from "../../data/dashboardPreviewSensor";
import { iconForSensorType, iconRobot, type SensorType } from "../../icons";
import { makeStatusStrip } from "../statusStrip";
import { subscribeHealth } from "../../data/health";
import type { TileHandle as StageTileHandle } from "../robot/tile";
import { makeK1PoseTile } from "../robot/tiles/k1Pose";
import { makePointCloudTile } from "../robot/tiles/pointCloud";
import { shortName } from "../robot/tiles/names";

type PillState = "live";

type PreviewKind = "camera" | "point_cloud" | "joint_encoders";

type PreviewOption = {
  sensor_id: string;
  kind: PreviewKind;
  entry?: SensorEntry;
};

export type CardHandle = {
  el: HTMLElement;
  dispose(): void;
};

export function makeLiveCard(d: Daemon): CardHandle {
  const el = makeCardShell(d.url, "live");
  // Daemons are cluster-sourced only (PARK-Q4); no operator-removable
  // entries. The Remove button is gone with the manual/mDNS sources.
  let disposeRemove: (() => void) | undefined;

  const thumbWrap = el.querySelector('[data-region="thumb"]')! as HTMLElement;
  const img = thumbWrap.querySelector("img")! as HTMLImageElement;
  const iconFallback = thumbWrap.querySelector('[data-region="icon"]')! as HTMLElement;
  const stateEl = el.querySelector('[data-region="state"]')!;
  const recEl = el.querySelector('[data-region="recording"]')!;
  const managerPill = el.querySelector('[data-region="manager-pill"]') as HTMLElement;
  const sensorSelect = el.querySelector('[data-region="thumb-sensor"]') as HTMLSelectElement;
  const sensorSelectWrap = el.querySelector('[data-region="thumb-sensor-wrap"]') as HTMLElement;

  setIdentity(el, d.name, d.app, d.url);
  setStatusPill(thumbWrap, "live");

  // Manager pill toggles based on cluster status. `daemon.url` is the
  // peer-id (cluster source); compare against `manager_peer_id` from
  // `/api/cluster/status`.
  const disposeManager = subscribeCluster((snap) => {
    const isManager =
      snap.status?.source.kind === "in_cluster" &&
      snap.status.source.manager_peer_id === d.url;
    managerPill.classList.toggle("hidden", !isManager);
  });

  // /api/info — paints session age into the pill once it resolves.
  // Failures here are NOT signalled at the card level; the directory
  // promotes failing cards to the unreachable variant on its own.
  let infoSnap: InfoSnapshot | null = null;
  const repaintInfo = () => {
    if (infoSnap) {
      const ageMs = Math.max(0, Date.now() - infoSnap.sessionStartedEstMs);
      setStatusPill(thumbWrap, "live", formatAge(ageMs));
    } else {
      setStatusPill(thumbWrap, "live");
    }
  };
  const ageTimer = window.setInterval(repaintInfo, 1000);

  // Dashboard preview surface. Camera streams use the shared JPEG preview
  // source; point clouds and pose streams reuse the existing detail-view
  // tile renderers with their chrome hidden so the card can preview the
  // same sensor modalities the robot stage can render.
  let unloaded = false;
  let lastPeerId: string | null = null;
  let lastKey: string | null = null;
  let unsubscribePreview: (() => void) | null = null;
  let activePreviewTile: StageTileHandle | null = null;
  let previewOptions: PreviewOption[] = [];
  let selectedSensorId: string | null = getDashboardPreviewSensor(d.url);
  let catalogInFlight = false;

  const selectedOrFallback = (): PreviewOption | null => {
    if (
      selectedSensorId &&
      previewOptions.some((o) => o.sensor_id === selectedSensorId)
    ) {
      return previewOptions.find((o) => o.sensor_id === selectedSensorId) ?? null;
    }
    return previewOptions[0] ?? null;
  };

  const paintSensorSelect = () => {
    const selected = selectedOrFallback();
    sensorSelect.replaceChildren();
    if (previewOptions.length === 0) {
      const opt = document.createElement("option");
      opt.value = "";
      opt.textContent = "No visual sensors";
      sensorSelect.appendChild(opt);
      sensorSelect.disabled = true;
      sensorSelectWrap.classList.add("opacity-60");
      paintPreviewPlaceholder(null);
      return;
    }
    sensorSelect.disabled = false;
    sensorSelectWrap.classList.remove("opacity-60");
    for (const option of previewOptions) {
      const opt = document.createElement("option");
      opt.value = option.sensor_id;
      opt.textContent = `${shortName(option.sensor_id)} - ${previewKindLabel(option.kind)}`;
      opt.title = option.sensor_id;
      sensorSelect.appendChild(opt);
    }
    sensorSelect.value = selected?.sensor_id ?? "";
  };

  const applyPreviewSensor = () => {
    paintSensorSelect();
    renewPreview();
  };

  const clearTilePreview = () => {
    activePreviewTile?.dispose();
    activePreviewTile?.el.remove();
    activePreviewTile = null;
  };

  const paintPreviewPlaceholder = (option: PreviewOption | null) => {
    const type = option ? previewSensorType(option.kind) : "sensor";
    const label = option ? previewKindLabel(option.kind) : "No preview";
    iconFallback.innerHTML = `
      <div class="flex flex-col items-center justify-center gap-1 text-paper/35">
        <span>${iconForSensorType(type, 38)}</span>
        <span class="text-[10px] uppercase tracking-[0.16em]" style="font-family: var(--font-display)">${escapeHtml(label)}</span>
      </div>
    `;
  };

  const mountTilePreview = (option: PreviewOption, peerId: string) => {
    clearTilePreview();
    const tile =
      option.kind === "point_cloud"
        ? makePointCloudTile(
            {
              sensor_id: option.sensor_id,
              daemon_url: d.url,
              peer_id: peerId,
              entry: option.entry,
            },
            { onClose: () => {} },
          )
        : makeK1PoseTile(
            {
              sensor_id: option.sensor_id,
              daemon_url: d.url,
              peer_id: peerId,
            },
            { onClose: () => {} },
          );
    activePreviewTile = tile;
    tile.el.className =
      "absolute inset-0 w-full h-full bg-ink overflow-hidden rounded-none border-0";
    tile.el.style.pointerEvents = "none";
    hideTileChrome(tile.el);
    thumbWrap.insertBefore(tile.el, img);
    img.style.opacity = "0";
    iconFallback.style.opacity = "0";
  };

  const renewPreview = () => {
    if (unloaded) return;
    const selected = selectedOrFallback();
    const key =
      selected
        ? `${lastPeerId ?? "pending"}::${selected.kind}::${selected.sensor_id}`
        : null;
    if (key === lastKey) return;
    lastKey = key;
    unsubscribePreview?.();
    unsubscribePreview = null;
    clearTilePreview();
    img.removeAttribute("src");
    img.style.opacity = "0";
    iconFallback.style.opacity = "1";
    paintPreviewPlaceholder(selected);
    if (!lastPeerId || !selected) {
      return;
    }
    if (selected.kind !== "camera") {
      mountTilePreview(selected, lastPeerId);
      return;
    }
    unsubscribePreview = subscribePreview(
      { peer_id: lastPeerId, sensor_id: selected.sensor_id },
      (frame) => {
        if (unloaded || !frame) return;
        img.src = frame.url;
        img.style.opacity = "1";
        iconFallback.style.opacity = "0";
      },
    );
  };

  const refreshCatalog = () => {
    if (catalogInFlight || unloaded) return;
    catalogInFlight = true;
    void fetchCatalog(d.url)
      .then((catalog) => {
        if (unloaded || !catalog) return;
        const next = catalog.sensors
          .map(previewOptionFromCatalog)
          .filter((o): o is PreviewOption => o !== null)
          .sort(sortPreviewOptions);
        const same =
          next.length === previewOptions.length &&
          next.every(
            (o, i) =>
              o.sensor_id === previewOptions[i]?.sensor_id &&
              o.kind === previewOptions[i]?.kind,
          );
        if (same) return;
        previewOptions = next;
        applyPreviewSensor();
      })
      .finally(() => {
        catalogInFlight = false;
      });
  };

  sensorSelect.addEventListener("click", (e) => e.stopPropagation());
  sensorSelect.addEventListener("change", () => {
    selectedSensorId = sensorSelect.value || null;
    setDashboardPreviewSensor(d.url, selectedSensorId);
    applyPreviewSensor();
  });
  paintSensorSelect();
  refreshCatalog();

  const disposeInfo = subscribeInfo(d.url, (snap) => {
    infoSnap = snap;
    repaintInfo();
    if (!snap) return;
    lastPeerId = snap.info.peer_id;
    renewPreview();
  });

  const disposeState = subscribeSensorLogs(d.url, (state) => {
    paintState(stateEl, recEl, state);
    refreshCatalog();
  });

  // ─── health timeline strip ─────────────────────────────────────────
  const healthStrip = makeStatusStrip({
    width: 240,
    height: 4,
    slots: 30,
    windowMs: 5 * 60 * 1000,
  });
  healthStrip.el.style.width = "100%";
  const stripSvg = healthStrip.el.querySelector("svg");
  if (stripSvg) {
    stripSvg.setAttribute("width", "100%");
    stripSvg.setAttribute("preserveAspectRatio", "none");
  }
  const stripWrap = document.createElement("div");
  stripWrap.className =
    "absolute bottom-0 inset-x-0 h-1 pointer-events-auto z-10";
  stripWrap.title = "Last 5 minutes — daemon /api/info health";
  stripWrap.appendChild(healthStrip.el);
  thumbWrap.appendChild(stripWrap);

  const disposeHealth = subscribeHealth(d.url, (samples) => {
    healthStrip.setSamples(samples);
  });

  return {
    el,
    dispose() {
      unloaded = true;
      unsubscribePreview?.();
      clearTilePreview();
      disposeState();
      disposeInfo();
      disposeHealth();
      disposeManager();
      clearInterval(ageTimer);
      disposeRemove?.();
    },
  };
}

function previewOptionFromCatalog(entry: SensorCatalogEntry): PreviewOption | null {
  const sensorEntry = sensorEntryFromCatalogEntry(entry);
  const kind = previewKindFromCatalog(entry, sensorEntry);
  if (!kind) {
    return null;
  }
  return {
    sensor_id: entry.sensor_id,
    kind,
    entry: sensorEntry,
  };
}

function previewKindFromCatalog(
  entry: SensorCatalogEntry,
  sensorEntry: SensorEntry | undefined,
): PreviewKind | null {
  if (entry.kind === "joint_encoders" || entry.sensor_id.endsWith("/joint_encoders")) {
    return "joint_encoders";
  }
  if (entry.kind === "camera" || sensorEntry?.type === "camera") {
    return "camera";
  }
  if (entry.kind === "point_cloud" || sensorEntry?.type === "point_cloud") {
    return "point_cloud";
  }
  return null;
}

function sortPreviewOptions(a: PreviewOption, b: PreviewOption): number {
  const ak = previewKindOrder(a.kind);
  const bk = previewKindOrder(b.kind);
  return ak - bk || a.sensor_id.localeCompare(b.sensor_id);
}

function previewKindOrder(kind: PreviewKind): number {
  switch (kind) {
    case "camera":
      return 0;
    case "point_cloud":
      return 1;
    case "joint_encoders":
      return 2;
  }
}

function previewKindLabel(kind: PreviewKind): string {
  switch (kind) {
    case "camera":
      return "Camera";
    case "point_cloud":
      return "Point cloud";
    case "joint_encoders":
      return "Pose";
  }
}

function previewSensorType(kind: PreviewKind): SensorType {
  switch (kind) {
    case "camera":
      return "camera";
    case "point_cloud":
      return "pointcloud";
    case "joint_encoders":
      return "pose";
  }
}

function hideTileChrome(el: HTMLElement) {
  for (const region of ["tile-top-left", "tile-top-right", "tile-bottom-bar"]) {
    const child = el.querySelector(`[data-region="${region}"]`) as HTMLElement | null;
    child?.classList.add("hidden");
  }
}

// ─── shared shell ────────────────────────────────────────────────────────────

function makeCardShell(url: string, _variant: PillState): HTMLElement {
  const base =
    "group text-left bg-ink-alt border rounded-md overflow-hidden flex flex-col cursor-pointer";
  const el = document.createElement("article");
  el.className = `${base} border-paper/10 hover:border-accent/40 transition-colors`;
  el.tabIndex = 0;
  el.setAttribute("role", "button");
  el.addEventListener("click", (e) => {
    if (isCardControl(e.target)) return;
    navigate({ view: "robot", url });
  });
  el.addEventListener("keydown", (e) => {
    if (isCardControl(e.target)) return;
    if (e.key !== "Enter" && e.key !== " ") return;
    e.preventDefault();
    navigate({ view: "robot", url });
  });

  el.innerHTML = `
    <div class="relative w-full aspect-video bg-ink overflow-hidden" data-region="thumb">
      <img class="absolute inset-0 w-full h-full object-cover opacity-0 transition-opacity duration-300" alt="" />
      <div class="absolute inset-0 flex items-center justify-center text-paper/30 transition-colors" data-region="icon">${iconRobot(48)}</div>
    </div>
    <div class="px-3 pt-2.5 pb-3">
      <div class="flex items-center gap-2 mb-0.5">
        <div class="text-paper text-sm font-medium truncate" data-region="name">—</div>
        <span data-region="manager-pill" class="hidden shrink-0 px-1.5 py-0.5 text-[10px] uppercase tracking-[0.15em] bg-accent/15 text-accent rounded-sm" style="font-family: var(--font-display)">manager</span>
      </div>
      <div class="text-rule text-xs mb-2" data-region="app">—</div>
      <div class="flex items-center justify-between text-[12px]">
        <span class="text-paper/70" data-region="state">— sensors</span>
        <span class="text-rule" data-region="recording">idle</span>
      </div>
      <div class="mt-2 flex items-center gap-2" data-region="thumb-sensor-wrap">
        <span class="text-rule/55 text-[10px] uppercase tracking-[0.12em] shrink-0" style="font-family: var(--font-display)">View</span>
        <select data-region="thumb-sensor" data-card-control="true" title="Dashboard sensor" class="min-w-0 flex-1 bg-ink border border-paper/10 rounded-sm px-2 py-1 text-[11px] text-paper/85 outline-none cursor-pointer focus:border-accent/60 disabled:cursor-default disabled:text-rule/70"></select>
      </div>
      <div class="flex items-center justify-between gap-2 mt-2 opacity-0 group-hover:opacity-100 transition-opacity">
        <div class="text-rule/40 text-[11px] truncate font-mono flex-1 min-w-0" data-region="url">—</div>
        <span class="text-[11px] shrink-0" data-region="remove-slot"></span>
      </div>
    </div>
  `;
  return el;
}

function isCardControl(target: EventTarget | null): boolean {
  return target instanceof HTMLElement && target.closest("[data-card-control]") !== null;
}

function setIdentity(
  el: HTMLElement,
  name: string,
  app: string,
  url: string,
) {
  el.querySelector('[data-region="name"]')!.textContent = name;
  el.querySelector('[data-region="app"]')!.textContent = app;
  el.querySelector('[data-region="url"]')!.textContent = url;
}

/** Single status pill, top-right of the thumbnail. Three states share
 * one rendering surface so the cards read as siblings. Labels are
 * uppercased; data values (age, last-seen) keep natural casing. */
function setStatusPill(thumbWrap: HTMLElement, state: PillState, detail?: string) {
  thumbWrap.querySelector('[data-region="pill"]')?.remove();
  const pill = document.createElement("div");
  pill.dataset.region = "pill";
  const baseClass =
    "absolute top-2 right-2 flex items-center gap-1.5 px-2 py-0.5 bg-ink/80 backdrop-blur-sm rounded-sm text-[11px]";
  const labelClass = "uppercase tracking-[0.12em] font-medium";

  if (state === "live") {
    pill.className = `${baseClass} border border-accent/40`;
    pill.innerHTML = detail
      ? `<span class="w-1.5 h-1.5 rounded-full bg-accent animate-pulse"></span><span class="text-paper/85 ${labelClass}">Live</span><span class="text-rule/60">·</span><span class="text-paper/85 font-mono">${escapeHtml(detail)}</span>`
      : `<span class="w-1.5 h-1.5 rounded-full bg-accent animate-pulse"></span><span class="text-paper/85 ${labelClass}">Live</span>`;
  } else if (state === "unreachable") {
    pill.className = `${baseClass} border border-rule/30 text-rule/85`;
    pill.innerHTML = `<span class="w-1.5 h-1.5 rounded-full bg-rule/60"></span><span class="${labelClass}">Unreachable</span>`;
  } else {
    pill.className = `${baseClass} border border-rule/30 text-rule/85`;
    pill.innerHTML = detail
      ? `<span class="w-1.5 h-1.5 rounded-full bg-rule/60"></span><span class="${labelClass}">Offline</span><span class="text-rule/60">·</span><span>${escapeHtml(detail)}</span>`
      : `<span class="w-1.5 h-1.5 rounded-full bg-rule/60"></span><span class="${labelClass}">Offline</span>`;
  }
  thumbWrap.appendChild(pill);
}


function paintState(
  stateEl: Element,
  recEl: Element,
  state: DaemonSensorLogs | null,
) {
  if (!state) {
    stateEl.textContent = "— sensors";
    recEl.textContent = "—";
    return;
  }
  const sensorIds = new Set<string>();
  let activeRecording = false;
  for (const l of state.sensor_logs) {
    sensorIds.add(l.sensor_id);
    if (l.retention_ns === 0 && l.stopped_at_ns == null) activeRecording = true;
  }
  const n = sensorIds.size;
  stateEl.textContent = `${n} sensor${n === 1 ? "" : "s"}`;
  if (activeRecording) {
    recEl.innerHTML = `<span class="inline-flex items-center gap-1 text-accent"><span class="w-1.5 h-1.5 rounded-full bg-accent animate-pulse"></span>recording</span>`;
  } else {
    recEl.textContent = "idle";
  }
}
