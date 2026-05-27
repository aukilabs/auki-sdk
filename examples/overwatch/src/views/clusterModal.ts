// Cluster participants modal. Opened from the topbar; shows the same
// participant data as the full cluster view but as a focused overlay so
// the operator never loses their place in the directory.

import { escapeHtml } from "../util/escape";
import { formatAge } from "../data/info";
import {
  subscribeCluster,
  type Participant,
  type ClusterStatus,
} from "../data/cluster";
import { getDaemons } from "../data/daemons";
import { navigate } from "../data/router";
import { openDomainPromptModal } from "./onboarding/domainPromptModal";

let open = false;

export function openClusterModal(): void {
  if (open) return;
  open = true;

  const overlay = document.createElement("div");
  overlay.className =
    "fixed inset-0 z-50 flex items-start justify-center bg-ink/75 backdrop-blur-sm overflow-y-auto py-16 px-4";

  const panel = document.createElement("div");
  panel.className =
    "relative w-full max-w-3xl rounded-md border border-paper/15 bg-ink-alt shadow-2xl flex flex-col";
  overlay.appendChild(panel);

  const closeBtn = document.createElement("button");
  closeBtn.className =
    "absolute top-3 right-3 w-7 h-7 flex items-center justify-center rounded-sm hover:bg-paper/8 text-rule/70 hover:text-paper transition-colors text-base leading-none z-10";
  closeBtn.title = "Close (Esc)";
  closeBtn.setAttribute("aria-label", "Close");
  closeBtn.textContent = "×";
  panel.appendChild(closeBtn);

  const close = () => {
    overlay.remove();
    disposeCluster();
    clearInterval(tickTimer);
    open = false;
  };

  overlay.addEventListener("click", (e) => {
    if (e.target === overlay) close();
  });
  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Escape") {
      close();
      document.removeEventListener("keydown", onKey);
    }
  };
  document.addEventListener("keydown", onKey);

  const header = document.createElement("div");
  header.className =
    "px-6 pt-5 pb-4 pr-12 border-b border-paper/10 flex items-center justify-between gap-4 shrink-0";
  header.innerHTML = `
    <div class="min-w-0">
      <div class="text-paper text-base font-medium" style="font-family: var(--font-display)">Cluster participants</div>
      <div class="text-rule/60 text-[11px] font-mono mt-0.5" data-region="status">—</div>
    </div>
    <button
      class="shrink-0 text-[11px] uppercase tracking-[0.15em] px-2.5 py-1 rounded-sm border border-paper/15 hover:border-paper/30 text-rule hover:text-paper transition-colors"
      style="font-family: var(--font-display)"
      data-region="switch-domain"
    >Switch domain</button>
  `;
  panel.appendChild(header);

  const statusEl = header.querySelector('[data-region="status"]') as HTMLElement;
  const switchBtn = header.querySelector('[data-region="switch-domain"]') as HTMLButtonElement;
  closeBtn.addEventListener("click", close);

  const body = document.createElement("div");
  body.className = "flex flex-col overflow-hidden";
  body.innerHTML = `
    <div class="overflow-x-auto">
      <table class="w-full text-[12px]">
        <thead>
          <tr class="grid grid-cols-[minmax(0,2fr)_1fr_1.4fr_1fr_1fr_70px] gap-3 px-5 py-2.5 bg-ink/50 border-b border-paper/10 text-rule text-[11px] uppercase tracking-[0.18em]" style="font-family: var(--font-display)">
            <th class="text-left font-normal">Peer</th>
            <th class="text-left font-normal">App</th>
            <th class="text-left font-normal">Instance</th>
            <th class="text-left font-normal">In cluster</th>
            <th class="text-left font-normal">In session</th>
            <th class="text-left font-normal sr-only">Actions</th>
          </tr>
        </thead>
        <tbody data-region="rows"></tbody>
      </table>
    </div>
    <div class="px-6 py-3 border-t border-paper/5 text-rule/55 text-[11px] leading-relaxed">
      As devices join the same domain as you, they'll appear here automatically.
    </div>
  `;
  panel.appendChild(body);

  const rowsEl = body.querySelector('[data-region="rows"]') as HTMLElement;

  let self: Participant | null = null;
  let peers: Participant[] = [];
  let clusterStatus: ClusterStatus | null = null;

  const urlForPeerId = (peerId: string): string | null => {
    // Cluster-driven: `daemons[].url` IS the peer-id under PARK-Q4.
    const live = getDaemons().find((d) => d.url === peerId);
    return live ? live.url : null;
  };

  const repaint = () => {
    rowsEl.replaceChildren();

    if (!self && peers.length === 0) {
      renderEmpty();
      updateStatus();
      return;
    }

    if (self) rowsEl.appendChild(makeRow(self));
    for (const p of peers) rowsEl.appendChild(makeRow(p));
    updateStatus();
  };

  const updateStatus = () => {
    if (!clusterStatus) {
      statusEl.textContent = "loading…";
      return;
    }
    const source = clusterStatus.source;
    if (source.kind === "not_in_cluster") {
      statusEl.textContent = "no domain selected";
    } else {
      const n = (self ? 1 : 0) + peers.length;
      statusEl.textContent = `${n} connected · ${source.cluster_name}`;
    }
  };

  const renderEmpty = () => {
    const source = clusterStatus?.source;
    const tr = document.createElement("tr");
    tr.className = "block";
    if (!source || source.kind === "not_in_cluster") {
      tr.innerHTML = `
        <td colspan="5" class="block px-5 py-10 text-center">
          <p class="text-paper/85 text-sm mb-1.5" style="font-family: var(--font-display)">No domain selected</p>
          <p class="text-rule/65 text-[12px] max-w-sm mx-auto">Park needs a Discovery URL and a Domain name before peers can join.</p>
        </td>`;
    } else {
      tr.innerHTML = `
        <td colspan="5" class="block px-5 py-10 text-center">
          <div class="flex items-center justify-center gap-2 mb-2 text-accent">
            <span class="w-1.5 h-1.5 rounded-full bg-accent animate-pulse"></span>
            <span class="text-[11px] uppercase tracking-[0.2em]" style="font-family: var(--font-display)">listening</span>
          </div>
          <p class="text-paper/80 text-sm mb-1" style="font-family: var(--font-display)">No participants yet</p>
          <p class="text-rule/65 text-[12px]">Waiting for peers to register under <span class="font-mono">${escapeHtml(source.cluster_name)}</span>.</p>
        </td>`;
    }
    rowsEl.appendChild(tr);
  };

  const tickAges = () => {
    rowsEl.querySelectorAll<HTMLElement>("[data-row]").forEach((row) => {
      const peerId = row.dataset.row!;
      const p = [self, ...peers].find((x) => x?.peer_id === peerId);
      if (!p) return;
      const inCluster = row.querySelector('[data-col="in-cluster"]');
      if (inCluster) inCluster.innerHTML = formatInCluster(p);
      const inSession = row.querySelector('[data-col="in-session"]');
      if (inSession) inSession.innerHTML = formatInSession(p);
    });
  };

  switchBtn.addEventListener("click", () => {
    const source = clusterStatus?.source;
    openDomainPromptModal({
      initialName: source?.kind === "in_cluster" ? source.cluster_name : "",
      initialDiscoveryUrl:
        source?.kind === "in_cluster"
          ? source.url
          : clusterStatus?.discovery_url ?? "",
    });
  });

  const disposeCluster = subscribeCluster((snap) => {
    self = snap.self;
    peers = snap.peers;
    clusterStatus = snap.status;
    repaint();
    tickAges();
  });
  const tickTimer = window.setInterval(tickAges, 1000);

  document.body.appendChild(overlay);

  function makeRow(p: Participant): HTMLElement {
    const tr = document.createElement("tr");
    tr.dataset.row = p.peer_id;
    const isSelf = p.kind === "self";
    tr.className =
      "grid grid-cols-[minmax(0,2fr)_1fr_1.4fr_1fr_1fr_70px] gap-3 px-5 py-3 border-t border-paper/5 text-[12px] items-start" +
      (isSelf ? " bg-accent/5" : "");

    const appHtml = `<span class="text-paper/85">${escapeHtml(p.info.app)}</span>`;
    const navigateUrl = isSelf ? null : urlForPeerId(p.peer_id);

    tr.innerHTML = `
      <td class="flex items-start min-w-0">
        <div class="min-w-0 flex-1">
          <div class="flex items-center gap-1.5 min-w-0">
            <span class="w-1.5 h-1.5 rounded-full shrink-0 ${isSelf ? "bg-accent animate-pulse" : "bg-rule/60"}"></span>
            <span class="text-paper font-mono text-[11px] shrink-0" data-region="peer-id" title="${escapeHtml(p.peer_id)}">${escapeHtml(middleTruncate(p.peer_id))}</span>
            <button class="shrink-0 text-rule/50 hover:text-paper transition-colors" data-region="copy-btn" title="Copy peer ID">
              <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="9" width="13" height="13" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg>
            </button>
          </div>
          <div class="text-rule/70 text-[11px] truncate pl-3">${escapeHtml(p.info.name)}${isSelf ? " · this device" : ""}</div>
        </div>
      </td>
      <td>${appHtml}</td>
      <td class="text-paper/70 font-mono text-[11px]">${escapeHtml(p.info.app_instance)}</td>
      <td data-col="in-cluster">${formatInCluster(p)}</td>
      <td class="text-paper/85" data-col="in-session">${formatInSession(p)}</td>
      <td class="flex items-start justify-end">
        ${navigateUrl
          ? `<button class="text-[11px] px-2 py-0.5 rounded-sm border border-paper/20 hover:border-accent/60 text-rule hover:text-accent transition-colors" data-region="goto" data-url="${escapeHtml(navigateUrl)}">Go →</button>`
          : ""}
      </td>
    `;

    tr.querySelector('[data-region="copy-btn"]')?.addEventListener("click", () => {
      void navigator.clipboard.writeText(p.peer_id).then(() => {
        const btn = tr.querySelector('[data-region="copy-btn"]') as HTMLElement;
        btn.innerHTML = `<svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" class="text-accent"><polyline points="20 6 9 17 4 12"/></svg>`;
        setTimeout(() => {
          btn.innerHTML = `<svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="9" width="13" height="13" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg>`;
        }, 1500);
      });
    });

    tr.querySelector('[data-region="goto"]')?.addEventListener("click", () => {
      const url = (tr.querySelector('[data-region="goto"]') as HTMLElement).dataset.url!;
      close();
      navigate({ view: "robot", url });
    });

    return tr;
  }
}

function formatInCluster(p: Participant): string {
  if (p.clusterJoinedEstMs == null) return `<span class="text-rule/65 italic">alone</span>`;
  const ageMs = Math.max(0, Date.now() - p.clusterJoinedEstMs);
  return `<span class="text-accent/90">${escapeHtml(formatAge(ageMs))}</span>`;
}

function formatInSession(p: Participant): string {
  const ageMs = Math.max(0, Date.now() - p.sessionStartedEstMs);
  return escapeHtml(formatAge(ageMs));
}

function middleTruncate(s: string, head = 12, tail = 6): string {
  if (s.length <= head + tail + 1) return s;
  return `${s.slice(0, head)}…${s.slice(-tail)}`;
}
