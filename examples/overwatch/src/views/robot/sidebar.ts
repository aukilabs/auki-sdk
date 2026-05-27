// Robot sidebar — left rail of the robot detail view, full-height.
// Live-streaming-only since the past-session viewer was retired.
//
// Two vertical zones:
//   1. Identity header — back arrow + device name. Operator's anchor:
//      they always know which device they're inspecting and have a
//      single-click escape back to the dashboard.
//   2. File structure tree — disk-shape view of the daemon's live
//      session. See `structure.ts`.

import { escapeHtml } from "../../util/escape";
import type { Daemon } from "../../data/daemons";
import type { DaemonSensorLogs } from "../../data/sensorLogs";
import { makeStructure } from "./structure";

export type SidebarHandle = {
  el: HTMLElement;
  setSensorLogs(state: DaemonSensorLogs | null): void;
  dispose(): void;
};

export function makeSidebar(d: Daemon | undefined): SidebarHandle {
  const el = document.createElement("aside");
  el.className =
    "row-start-1 col-start-1 border-r border-paper/10 bg-ink overflow-hidden flex flex-col min-h-0";

  if (!d) {
    el.innerHTML = `
      <div class="px-5 py-6 flex flex-col gap-4 h-full">
        <div class="px-4 py-6 text-center border border-dashed border-paper/10 rounded-md flex-1 flex flex-col items-center justify-center gap-2">
          <p class="text-paper/85 text-sm" style="font-family: var(--font-display)">Device not found</p>
          <p class="text-rule/70 text-[12px] leading-snug">It may have gone offline or the URL is no longer valid.</p>
        </div>
        <a href="#/" class="text-rule hover:text-paper text-xs text-center transition-colors">← Back to dashboard</a>
      </div>
    `;
    return {
      el,
      setSensorLogs: () => {},
      dispose: () => {},
    };
  }

  // ─── identity header ───────────────────────────────────────────────
  // `< name` — the chevron is a back-link to the dashboard. No status
  // line below: connection state is implicit in the file structure
  // tree (which surfaces "Waiting for daemon" when offline).
  const header = document.createElement("div");
  header.className = "px-5 pt-5 pb-3 shrink-0";
  header.innerHTML = `
    <div class="flex items-center gap-2 min-w-0">
      <a href="#/" title="Back to dashboard"
         class="text-rule/70 hover:text-paper transition-colors shrink-0 -ml-1 px-1 py-0.5 rounded-sm hover:bg-ink-alt/60">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M15 18l-6-6 6-6"/>
        </svg>
      </a>
      <h2 class="text-paper text-lg font-medium leading-tight truncate" style="font-family: var(--font-display)" title="${escapeHtml(d.name)}">${escapeHtml(d.name)}</h2>
    </div>
  `;
  el.appendChild(header);

  // ─── file structure section ────────────────────────────────────────
  const structureHeader = document.createElement("div");
  structureHeader.className =
    "px-5 pt-3 pb-2 flex items-center justify-between border-t border-paper/10 shrink-0";
  structureHeader.innerHTML = `
    <span class="text-rule text-[11px] uppercase tracking-[0.2em]" style="font-family: var(--font-display)">File structure</span>
    <span class="text-rule/60 text-[11px]">live</span>
  `;
  el.appendChild(structureHeader);

  const structure = makeStructure(d);
  el.appendChild(structure.el);

  return {
    el,
    setSensorLogs() {
      // Structure tree subscribes to sensor_logs itself; nothing to
      // forward here. The hook stays on the handle in case a future
      // header pill wants to surface live-session status.
    },
    dispose() {
      structure.dispose();
    },
  };
}
