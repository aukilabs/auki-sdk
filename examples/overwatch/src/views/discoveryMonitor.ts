import { fetchDiscoverySnapshot, type DiscoverySnapshot } from "../data/discovery";
import { formatAge } from "../data/info";
import { subscribeCluster, shortPeer, type ClusterStatus } from "../data/cluster";
import { iconCopy, iconDatabase, iconRefresh } from "../icons";
import { escapeHtml } from "../util/escape";
import { openDomainPromptModal } from "./onboarding/domainPromptModal";

let open = false;

export function openDiscoveryMonitor(): void {
  if (open) return;
  open = true;

  const overlay = document.createElement("div");
  overlay.className =
    "fixed inset-0 z-50 flex items-start justify-center bg-ink/75 backdrop-blur-sm overflow-y-auto py-12 px-4";

  const panel = document.createElement("div");
  panel.className =
    "relative w-full max-w-6xl rounded-md border border-paper/15 bg-ink-alt shadow-2xl flex flex-col overflow-hidden";
  overlay.appendChild(panel);

  panel.innerHTML = `
    <div class="px-5 pt-4 pb-3 pr-12 border-b border-paper/10 flex items-center gap-3 shrink-0">
      <span class="text-paper/60 shrink-0">${iconDatabase(16)}</span>
      <div class="min-w-0 flex-1">
        <div class="text-paper text-base font-medium" style="font-family: var(--font-display)">Discovery monitor</div>
        <div class="text-rule/60 text-[11px] font-mono truncate" data-region="status">loading...</div>
      </div>
      <button class="w-8 h-8 flex items-center justify-center rounded-sm border border-paper/10 text-rule hover:text-paper hover:border-paper/30 transition-colors" data-region="refresh" title="Refresh">
        ${iconRefresh(14)}
      </button>
      <button class="absolute top-3 right-3 w-7 h-7 flex items-center justify-center rounded-sm hover:bg-paper/8 text-rule/70 hover:text-paper transition-colors text-base leading-none" data-region="close" title="Close (Esc)" aria-label="Close">x</button>
    </div>
    <div class="grid grid-cols-2 md:grid-cols-4 border-b border-paper/10 bg-ink/40 text-[12px]" data-region="metrics">
      <div class="px-5 py-3 border-r border-paper/10 min-w-0">
        <div class="text-rule/65 text-[10px] uppercase" style="font-family: var(--font-display)">URL</div>
        <div class="text-paper/85 font-mono truncate mt-1" data-region="metric-url">-</div>
      </div>
      <div class="px-5 py-3 border-r border-paper/10 min-w-0">
        <div class="text-rule/65 text-[10px] uppercase" style="font-family: var(--font-display)">Clusters</div>
        <div class="text-paper/85 font-mono mt-1" data-region="metric-clusters">-</div>
      </div>
      <div class="px-5 py-3 border-r border-paper/10 min-w-0">
        <div class="text-rule/65 text-[10px] uppercase" style="font-family: var(--font-display)">Peers</div>
        <div class="text-paper/85 font-mono mt-1" data-region="metric-peers">-</div>
      </div>
      <div class="px-5 py-3 min-w-0">
        <div class="text-rule/65 text-[10px] uppercase" style="font-family: var(--font-display)">Updated</div>
        <div class="text-paper/85 font-mono mt-1" data-region="metric-updated">-</div>
      </div>
    </div>
    <div class="grid lg:grid-cols-[minmax(0,1fr)_minmax(320px,0.42fr)] min-h-[520px] max-h-[74vh] overflow-hidden">
      <section class="min-w-0 flex flex-col border-r border-paper/10">
        <div class="px-5 py-2.5 border-b border-paper/10 text-rule text-[11px] uppercase" style="font-family: var(--font-display)">Clusters</div>
        <div class="overflow-auto">
          <table class="w-full text-[12px]">
            <thead>
              <tr class="grid grid-cols-[minmax(190px,1.4fr)_80px_minmax(170px,1fr)_minmax(220px,1.3fr)_minmax(170px,1fr)] gap-3 px-5 py-2.5 bg-ink/35 border-b border-paper/10 text-rule text-[10px] uppercase" style="font-family: var(--font-display)">
                <th class="text-left font-normal">Domain</th>
                <th class="text-left font-normal">Peers</th>
                <th class="text-left font-normal">Manager</th>
                <th class="text-left font-normal">Multiaddrs</th>
                <th class="text-left font-normal">Liveness</th>
              </tr>
            </thead>
            <tbody data-region="rows"></tbody>
          </table>
        </div>
      </section>
      <section class="min-w-0 flex flex-col bg-ink/20">
        <div class="px-4 py-2.5 border-b border-paper/10 flex items-center gap-2">
          <div class="text-rule text-[11px] uppercase flex-1" style="font-family: var(--font-display)">Raw JSON</div>
          <button class="w-7 h-7 flex items-center justify-center rounded-sm border border-paper/10 text-rule hover:text-paper hover:border-paper/30 transition-colors disabled:opacity-40" data-region="copy" title="Copy raw JSON">
            ${iconCopy(13)}
          </button>
        </div>
        <pre class="m-0 p-4 overflow-auto text-[11px] leading-relaxed text-paper/80 font-mono whitespace-pre-wrap break-words" data-region="raw"></pre>
      </section>
    </div>
  `;

  const statusEl = panel.querySelector('[data-region="status"]') as HTMLElement;
  const refreshBtn = panel.querySelector('[data-region="refresh"]') as HTMLButtonElement;
  const closeBtn = panel.querySelector('[data-region="close"]') as HTMLButtonElement;
  const urlEl = panel.querySelector('[data-region="metric-url"]') as HTMLElement;
  const clustersEl = panel.querySelector('[data-region="metric-clusters"]') as HTMLElement;
  const peersEl = panel.querySelector('[data-region="metric-peers"]') as HTMLElement;
  const updatedEl = panel.querySelector('[data-region="metric-updated"]') as HTMLElement;
  const rowsEl = panel.querySelector('[data-region="rows"]') as HTMLElement;
  const rawEl = panel.querySelector('[data-region="raw"]') as HTMLElement;
  const copyBtn = panel.querySelector('[data-region="copy"]') as HTMLButtonElement;

  let clusterStatus: ClusterStatus | null = null;
  let snapshot: DiscoverySnapshot | null = null;
  let error: string | null = null;
  let loading = false;
  let fetchSeq = 0;
  let lastUrl: string | null = null;

  const close = () => {
    overlay.remove();
    document.removeEventListener("keydown", onKey);
    clearInterval(refreshTimer);
    disposeCluster();
    open = false;
  };

  const render = () => {
    const url = discoveryUrlFromStatus(clusterStatus);
    urlEl.textContent = url ?? "-";
    urlEl.title = url ?? "";
    clustersEl.textContent = snapshot ? String(snapshot.clusters.length) : "-";
    peersEl.textContent = snapshot
      ? String(snapshot.clusters.reduce((sum, c) => sum + c.peer_count, 0))
      : "-";
    updatedEl.textContent = snapshot
      ? `${formatAge(Date.now() - snapshot.fetched_at_unix_ms)} ago`
      : "-";

    refreshBtn.disabled = loading;
    copyBtn.disabled = !snapshot?.raw_json;

    if (!url) {
      statusEl.textContent = "no Discovery URL";
      rowsEl.innerHTML = emptyRow(
        "No Discovery URL",
        `<button class="mt-2 px-3 py-1.5 rounded-sm border border-paper/15 hover:border-paper/30 text-rule hover:text-paper transition-colors" data-region="set-domain">Set domain</button>`,
      );
      rawEl.textContent = "";
      rowsEl.querySelector('[data-region="set-domain"]')?.addEventListener("click", () => {
        close();
        openDomainPromptModal({ mandatory: false });
      });
      return;
    }

    if (error) {
      statusEl.textContent = `error | ${url}`;
      rowsEl.innerHTML = emptyRow("Discovery error", escapeHtml(error));
      rawEl.textContent = error;
      return;
    }

    if (loading && !snapshot) {
      statusEl.textContent = `loading | ${url}`;
      rowsEl.innerHTML = emptyRow("Loading", "");
      rawEl.textContent = "";
      return;
    }

    statusEl.textContent = snapshot
      ? `${snapshot.clusters.length} clusters | ${snapshot.discovery_url}`
      : `ready | ${url}`;
    rawEl.textContent = snapshot?.raw_json ?? "";
    renderRows(snapshot);
  };

  const refresh = async () => {
    const url = discoveryUrlFromStatus(clusterStatus);
    if (!url) {
      snapshot = null;
      error = null;
      loading = false;
      render();
      return;
    }

    const seq = ++fetchSeq;
    loading = true;
    error = null;
    render();
    try {
      const next = await fetchDiscoverySnapshot(url);
      if (seq !== fetchSeq) return;
      snapshot = next;
      error = null;
    } catch (err) {
      if (seq !== fetchSeq) return;
      error = err instanceof Error ? err.message : String(err);
    } finally {
      if (seq === fetchSeq) {
        loading = false;
        render();
      }
    }
  };

  const renderRows = (snap: DiscoverySnapshot | null) => {
    if (!snap) {
      rowsEl.innerHTML = emptyRow("No snapshot", "");
      return;
    }
    if (snap.clusters.length === 0) {
      rowsEl.innerHTML = emptyRow("0 clusters", "");
      return;
    }
    rowsEl.innerHTML = snap.clusters.map(clusterRow).join("");
  };

  const copyRaw = () => {
    if (!snapshot?.raw_json) return;
    void navigator.clipboard.writeText(snapshot.raw_json).then(() => {
      const prev = copyBtn.title;
      copyBtn.title = "Copied";
      setTimeout(() => {
        copyBtn.title = prev;
      }, 1200);
    });
  };

  closeBtn.addEventListener("click", close);
  refreshBtn.addEventListener("click", () => {
    lastUrl = discoveryUrlFromStatus(clusterStatus);
    void refresh();
  });
  copyBtn.addEventListener("click", copyRaw);
  overlay.addEventListener("click", (e) => {
    if (e.target === overlay) close();
  });
  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Escape") close();
  };
  document.addEventListener("keydown", onKey);

  const disposeCluster = subscribeCluster((snap) => {
    clusterStatus = snap.status;
    const nextUrl = discoveryUrlFromStatus(clusterStatus);
    if (nextUrl !== lastUrl) {
      lastUrl = nextUrl;
      void refresh();
    } else {
      render();
    }
  });
  const refreshTimer = window.setInterval(() => void refresh(), 2000);

  document.body.appendChild(overlay);
  render();
}

function discoveryUrlFromStatus(status: ClusterStatus | null): string | null {
  if (status?.source.kind === "in_cluster") return status.source.url;
  return status?.discovery_url ?? null;
}

function clusterRow(c: DiscoverySnapshot["clusters"][number]): string {
  const created = formatUnixNs(c.created_ns);
  const liveness = formatUnixNs(c.last_liveness_check_ns);
  const addrs =
    c.manager_multiaddrs.length > 0
      ? c.manager_multiaddrs
          .map(
            (addr) =>
              `<div class="font-mono text-[10px] text-paper/75 truncate" title="${escapeHtml(addr)}">${escapeHtml(addr)}</div>`,
          )
          .join("")
      : `<span class="text-rule/60 italic">none</span>`;

  return `
    <tr class="grid grid-cols-[minmax(190px,1.4fr)_80px_minmax(170px,1fr)_minmax(220px,1.3fr)_minmax(170px,1fr)] gap-3 px-5 py-3 border-t border-paper/5 items-start">
      <td class="min-w-0">
        <div class="font-mono text-paper truncate" title="${escapeHtml(c.name)}">${escapeHtml(c.name)}</div>
        <div class="text-rule/65 text-[10px] font-mono mt-1" title="${escapeHtml(String(c.created_ns))}">${escapeHtml(created)}</div>
      </td>
      <td class="font-mono text-paper/85">${escapeHtml(String(c.peer_count))}</td>
      <td class="font-mono text-paper/80 text-[11px] truncate" title="${escapeHtml(c.manager_peer_id)}">${escapeHtml(shortPeer(c.manager_peer_id))}</td>
      <td class="min-w-0 space-y-1">${addrs}</td>
      <td class="min-w-0">
        <div class="text-paper/80 text-[11px]">${escapeHtml(liveness)}</div>
        <div class="text-rule/65 text-[10px] font-mono mt-1" title="${escapeHtml(String(c.last_liveness_check_ns))}">${escapeHtml(String(c.last_liveness_check_ns))}</div>
      </td>
    </tr>
  `;
}

function emptyRow(title: string, body: string): string {
  return `
    <tr class="block">
      <td colspan="5" class="block px-5 py-14 text-center">
        <div class="text-paper/85 text-sm mb-1.5" style="font-family: var(--font-display)">${escapeHtml(title)}</div>
        ${body ? `<div class="text-rule/65 text-[12px]">${body}</div>` : ""}
      </td>
    </tr>
  `;
}

function formatUnixNs(ns: number): string {
  if (!Number.isFinite(ns) || ns <= 0) return "-";
  const ms = Math.floor(ns / 1_000_000);
  const date = new Date(ms);
  if (Number.isNaN(date.getTime())) return String(ns);
  const age = formatAge(Date.now() - ms);
  return `${date.toLocaleTimeString()} | ${age} ago`;
}
