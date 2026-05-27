// Directory banner — three-line header for the dashboard.
//
// Line 1 (kicker, accent): "Welcome to Park"
// Line 2 (heading): "Browsing cluster 'name'" / "No cluster configured"
// Line 3 (status):  "N robots · N sensors active · updated Xs ago [↺]"
//
// Auto-refresh: every 30 s the "updated" timestamp resets to now, even if
// no daemon state changed, so the operator gets a periodic "we're alive"
// signal. The Refresh button does the same on demand.

import { subscribeDaemons, type Daemon } from "../../data/daemons";
import { subscribeSensorLogs, type DaemonSensorLogs } from "../../data/sensorLogs";
import { subscribeCluster, type ClusterSnapshot } from "../../data/cluster";
import { subscribeMic, type MicSnapshot } from "../../data/mic";

const AUTO_REFRESH_MS = 30_000;

export type BannerHandle = {
  el: HTMLElement;
  dispose(): void;
};

export function makeBanner(): BannerHandle {
  const el = document.createElement("div");
  el.className = "px-6 pt-6 pb-4 select-none";
  el.innerHTML = `
    <div class="text-[11px] uppercase tracking-[0.18em] text-accent">Welcome to Park</div>
    <h1 class="text-[28px] leading-tight font-semibold text-paper mt-1" data-region="heading"></h1>
    <div class="mt-1 flex items-center gap-2 text-[13px] text-rule">
      <span data-region="status-line"></span>
      <button data-region="refresh-btn"
              class="ml-1 inline-flex items-center justify-center w-6 h-6 rounded hover:bg-paper/10 text-rule hover:text-paper transition-colors"
              title="Refresh status">
        <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2"
             stroke-linecap="round" stroke-linejoin="round">
          <path d="M21 12a9 9 0 1 1-2.64-6.36"></path>
          <polyline points="21 4 21 12 13 12"></polyline>
        </svg>
      </button>
    </div>
  `;

  const headingEl = el.querySelector('[data-region="heading"]') as HTMLElement;
  const statusEl = el.querySelector('[data-region="status-line"]') as HTMLElement;
  const refreshBtn = el.querySelector('[data-region="refresh-btn"]') as HTMLButtonElement;

  // ─── daemon state ──────────────────────────────────────────────────────
  let daemons: Daemon[] = [];
  const daemonStates = new Map<string, DaemonSensorLogs | null>();
  const stateUnsubs = new Map<string, () => void>();
  let lastUpdate = Date.now();
  let micSnap: MicSnapshot | null = null;

  const ensureStateSubsForDaemons = () => {
    const wanted = new Set(daemons.map((d) => d.url));
    for (const [url, unsub] of stateUnsubs) {
      if (!wanted.has(url)) { unsub(); stateUnsubs.delete(url); daemonStates.delete(url); }
    }
    for (const d of daemons) {
      if (stateUnsubs.has(d.url)) continue;
      const unsub = subscribeSensorLogs(d.url, (s) => {
        daemonStates.set(d.url, s);
        lastUpdate = Date.now();
        renderStatus();
      });
      stateUnsubs.set(d.url, unsub);
    }
  };

  // ─── cluster state ─────────────────────────────────────────────────────
  let clusterSnap: ClusterSnapshot | null = null;

  // ─── render functions (declared before subscriptions to avoid TDZ) ─────
  const renderHeading = () => {
    const source = clusterSnap?.status?.source;
    if (!source) {
      headingEl.textContent = "Connecting to Park…";
      return;
    }
    if (source.kind === "in_cluster") {
      headingEl.textContent = `Browsing domain '${source.cluster_name}'`;
    } else {
      headingEl.textContent = "No domain selected";
    }
  };

  const renderStatus = () => {
    const parkIsOnline = clusterSnap?.status?.source.kind === "in_cluster";
    const robotCount = daemons.length + (parkIsOnline ? 1 : 0);
    const sensorIds = new Set<string>();
    let activeRecordings = 0;
    for (const d of daemons) {
      const s = daemonStates.get(d.url);
      if (!s) continue;
      for (const l of s.sensor_logs) {
        sensorIds.add(`${d.url}::${l.sensor_id}`);
        if (l.retention_ns === 0 && l.stopped_at_ns == null) activeRecordings++;
      }
    }
    if (parkIsOnline && micSnap?.sensorId) {
      sensorIds.add(`park::${micSnap.sensorId}`);
    }

    const parts: string[] = [];
    if (robotCount === 0) {
      parts.push("scanning for robots…");
    } else {
      parts.push(`${robotCount} robot${robotCount === 1 ? "" : "s"} online`);
      if (sensorIds.size > 0) parts.push(`${sensorIds.size} sensor${sensorIds.size === 1 ? "" : "s"} active`);
      if (activeRecordings > 0) parts.push(`${activeRecordings} recording${activeRecordings === 1 ? "" : "s"}`);
    }
    parts.push(`updated ${secsAgo(lastUpdate)}s ago`);
    statusEl.textContent = parts.join(" · ");
  };

  // ─── subscriptions ─────────────────────────────────────────────────────
  const disposeCluster = subscribeCluster((snap) => {
    clusterSnap = snap;
    renderHeading();
    renderStatus();
  });

  const disposeDaemons = subscribeDaemons((next) => {
    daemons = next;
    lastUpdate = Date.now();
    ensureStateSubsForDaemons();
    renderStatus();
  });

  const disposeMic = subscribeMic((snap) => {
    micSnap = snap;
    renderStatus();
  });

  // Manual refresh — forces the "updated" stamp back to now.
  refreshBtn.addEventListener("click", () => {
    lastUpdate = Date.now();
    renderStatus();
  });

  // Auto-refresh every 30 s. If nothing's changed organically, the
  // counter resets so the operator sees fresh status, not creep-up.
  const autoRefreshTimer = window.setInterval(() => {
    lastUpdate = Date.now();
    renderStatus();
  }, AUTO_REFRESH_MS);

  // 1 s tick for the displayed counter (no data fetch, just text update).
  const tickInterval = window.setInterval(() => {
    statusEl.textContent = statusEl.textContent?.replace(
      /updated \d+s ago/,
      `updated ${secsAgo(lastUpdate)}s ago`,
    ) ?? "";
  }, 1_000);

  renderHeading();
  renderStatus();

  return {
    el,
    dispose() {
      disposeDaemons();
      disposeCluster();
      disposeMic();
      stateUnsubs.forEach((u) => u());
      stateUnsubs.clear();
      clearInterval(tickInterval);
      clearInterval(autoRefreshTimer);
    },
  };
}

function secsAgo(at: number): number {
  return Math.max(0, Math.round((Date.now() - at) / 1000));
}
