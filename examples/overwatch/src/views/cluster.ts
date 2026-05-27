// Cluster view (#/cluster) — the participant list.
//
// 5-column table per the ansuz doc:
//   PEER       — libp2p PeerId (short form)
//   APP        — `boosterapp`, `sentinel`, `park`
//   INSTANCE   — app_instance (per-machine MAC hex)
//   IN CLUSTER — time since this peer first connected to another peer.
//                Self-reported per ansuz D3 (peer.session_now_ns
//                − peer.cluster_joined_at_ns). null while the peer is
//                alone — rendered as "alone".
//   IN SESSION — time since the peer's session boot.
//
// Park itself appears first, distinguished with a `· this device` tag.
// Live ages tick once a second.

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
import { makeParkSelfWarnings } from "./parkSelfWarnings";

type View = { el: HTMLElement; dispose: () => void };

export function cluster(): View {
  const el = document.createElement("main");
  el.className = "flex-1 overflow-y-auto flex flex-col bg-ink-alt";

  const header = document.createElement("div");
  header.className =
    "px-8 pt-6 pb-4 border-b border-paper/10 flex items-start justify-between gap-4";
  header.innerHTML = `
    <div class="min-w-0">
      <div class="text-accent text-[11px] tracking-[0.3em] uppercase mb-1.5" style="font-family: var(--font-wordmark)">Cluster</div>
      <h1 class="text-paper text-2xl font-light leading-tight" style="font-family: var(--font-display)">Participants</h1>
      <p class="text-rule/70 text-[12px] mt-1.5 max-w-2xl leading-snug">
        Live libp2p exchange over <span class="font-mono">/auki/cluster/1.0.0</span> — every peer registered against
        Discovery introduces itself. Park is also a peer.
      </p>
      <div class="mt-3 text-[11px] font-mono text-rule/70 truncate max-w-2xl" data-region="domain-line" title="">—</div>
    </div>
    <div class="flex flex-col items-end gap-2 shrink-0">
      <div class="text-rule/60 text-[11px] font-mono" data-region="status">—</div>
      <button
        class="text-[11px] uppercase tracking-[0.15em] px-2.5 py-1 rounded-sm border border-paper/15 hover:border-paper/30 text-rule hover:text-paper transition-colors"
        style="font-family: var(--font-display)"
        data-region="switch-domain"
        title="Switch domain"
      >Switch domain</button>
    </div>
  `;
  el.appendChild(header);

  const tableWrap = document.createElement("div");
  tableWrap.className = "px-8 py-6 flex-1";
  tableWrap.innerHTML = `
    <div data-region="self-warnings"></div>
    <div class="rounded-md border border-paper/10 overflow-hidden">
      <div class="grid grid-cols-[2fr_1fr_1.4fr_1fr_1fr_auto] gap-4 px-4 py-2.5 bg-ink/50 border-b border-paper/10 text-rule text-[11px] uppercase tracking-[0.2em]" style="font-family: var(--font-display)">
        <div>Peer</div>
        <div>App</div>
        <div>Instance</div>
        <div>In cluster</div>
        <div>In session</div>
        <div></div>
      </div>
      <div data-region="rows"></div>
    </div>
    <p class="text-rule/55 text-[11px] mt-3 leading-snug max-w-2xl">
      Park sets its own <span class="font-mono">cluster_joined_at_ns</span> the first time any peer connects.
      "In cluster" is self-reported per ansuz D3 — what the peer reports about itself, not what Park observes.
    </p>
  `;
  el.appendChild(tableWrap);

  const selfWarningsSlot = tableWrap.querySelector(
    '[data-region="self-warnings"]',
  ) as HTMLElement;
  const selfWarningsBanner = makeParkSelfWarnings();
  selfWarningsSlot.appendChild(selfWarningsBanner.el);

  const rowsEl = tableWrap.querySelector('[data-region="rows"]') as HTMLElement;
  const statusEl = header.querySelector('[data-region="status"]') as HTMLElement;
  const domainLineEl = header.querySelector('[data-region="domain-line"]') as HTMLElement;
  const switchBtn = header.querySelector('[data-region="switch-domain"]') as HTMLButtonElement;

  let self: Participant | null = null;
  let peers: Participant[] = [];
  let clusterStatus: ClusterStatus | null = null;

  const renderEmpty = () => {
    const source = clusterStatus?.source;
    if (!source || source.kind === "not_in_cluster") {
      rowsEl.innerHTML = `
        <div class="px-6 py-10 text-center border-t border-paper/5">
          <p class="text-paper/85 text-sm mb-1.5" style="font-family: var(--font-display)">No domain selected</p>
          <p class="text-rule/65 text-[12px] max-w-md mx-auto leading-snug mb-4">
            Park needs a Discovery URL and a Domain name before it can exchange ParticipantInfo with peers.
          </p>
        </div>
      `;
      return;
    }
    rowsEl.innerHTML = `
      <div class="px-4 py-12 text-center border-t border-paper/5">
        <div class="flex items-center justify-center gap-2 mb-3 text-accent">
          <span class="w-2 h-2 rounded-full bg-accent animate-pulse"></span>
          <span class="text-[11px] uppercase tracking-[0.2em]" style="font-family: var(--font-display)">listening</span>
        </div>
        <p class="text-paper/80 text-sm mb-1" style="font-family: var(--font-display)">No participants yet</p>
        <p class="text-rule/65 text-[12px] max-w-md mx-auto leading-snug">
          Waiting for other peers to register under <span class="font-mono">${escapeHtml(source.cluster_name)}</span>.
        </p>
      </div>
    `;
  };

  const repaint = () => {
    rowsEl.replaceChildren();
    paintStatusChrome();

    if (!self && peers.length === 0) {
      renderEmpty();
      return;
    }

    if (self) rowsEl.appendChild(participantRow(self));
    for (const p of peers) rowsEl.appendChild(participantRow(p));
  };

  const paintStatusChrome = () => {
    if (!clusterStatus) {
      domainLineEl.textContent = "loading…";
      domainLineEl.title = "";
      return;
    }
    const source = clusterStatus.source;
    if (source.kind === "in_cluster") {
      domainLineEl.title = `${source.url} · ${source.cluster_name}`;
      domainLineEl.innerHTML = `<span class="text-accent uppercase tracking-[0.18em] text-[10px]">domain</span> <span class="text-rule/85">${escapeHtml(source.cluster_name)}</span> <span class="text-rule/55">· ${escapeHtml(source.url)}</span>`;
    } else {
      domainLineEl.title = "";
      domainLineEl.innerHTML = `<span class="text-rule/60 italic">no domain selected</span>`;
    }
  };

  const tickAges = () => {
    const rows = rowsEl.querySelectorAll<HTMLElement>("[data-row]");
    rows.forEach((row) => {
      const peerId = row.dataset.row!;
      const p = participantById(peerId);
      if (!p) return;
      const inCluster = row.querySelector('[data-col="in-cluster"]')!;
      const inSession = row.querySelector('[data-col="in-session"]')!;
      inCluster.innerHTML = formatInCluster(p);
      inSession.innerHTML = formatInSession(p);
    });
    if (self) {
      const ageMs = Math.max(0, Date.now() - self.receivedAtMs);
      const live = peers.length + 1;
      statusEl.textContent =
        ageMs < 2000
          ? `live · ${live} participant${live === 1 ? "" : "s"}`
          : `stale · ${Math.floor(ageMs / 1000)}s since refresh`;
    }
  };

  const participantById = (peerId: string): Participant | null => {
    if (self?.peer_id === peerId) return self;
    return peers.find((p) => p.peer_id === peerId) ?? null;
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

  const dispose = subscribeCluster((snap) => {
    self = snap.self;
    peers = snap.peers;
    clusterStatus = snap.status;
    selfWarningsBanner.setWarnings(snap.selfWarnings);
    repaint();
    tickAges();
  });
  const tickTimer = window.setInterval(tickAges, 1000);

  return {
    el,
    dispose: () => {
      dispose();
      clearInterval(tickTimer);
    },
  };
}

function participantRow(p: Participant): HTMLElement {
  const row = document.createElement("div");
  row.dataset.row = p.peer_id;
  const isSelf = p.kind === "self";
  const base =
    "grid grid-cols-[2fr_1fr_1.4fr_1fr_1fr_auto] gap-4 px-4 py-3 border-t border-paper/5 text-[12px] items-center";
  row.className = isSelf ? `${base} bg-accent/5` : base;

  const appCell = `<span class="text-paper/85">${escapeHtml(p.info.app)}</span>`;

  // Cluster-driven: `daemons[].url` IS the peer-id under PARK-Q4.
  const navigateUrl = !isSelf
    ? getDaemons().find((d) => d.url === p.peer_id)?.url ?? null
    : null;

  row.innerHTML = `
    <div class="flex items-center gap-2 min-w-0">
      ${isSelf
        ? `<span class="w-1.5 h-1.5 rounded-full bg-accent shrink-0 animate-pulse"></span>`
        : `<span class="w-1.5 h-1.5 rounded-full bg-rule/60 shrink-0"></span>`}
      <div class="min-w-0 flex-1">
        <div class="flex items-center gap-1.5 min-w-0">
          <span class="text-paper font-mono text-[11px] truncate min-w-0" data-region="peer-id" title="${escapeHtml(p.peer_id)}">${escapeHtml(p.peer_id)}</span>
          <button
            class="shrink-0 text-rule/50 hover:text-paper transition-colors"
            data-region="copy-btn"
            title="Copy peer ID"
          ><svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="9" width="13" height="13" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg></button>
        </div>
        <div class="text-rule/70 text-[11px] truncate">${escapeHtml(p.info.name)}${isSelf ? " · this device" : ""}</div>
      </div>
    </div>
    <div class="flex items-center">${appCell}</div>
    <div class="flex items-center text-paper/75 font-mono text-[11px]" title="${escapeHtml(p.info.app_instance)}">${escapeHtml(p.info.app_instance)}</div>
    <div class="flex items-center" data-col="in-cluster">${formatInCluster(p)}</div>
    <div class="flex items-center text-paper/85" data-col="in-session">${formatInSession(p)}</div>
    <div class="flex items-center justify-end">
      ${navigateUrl
        ? `<button class="text-[11px] px-2 py-0.5 rounded-sm border border-paper/20 hover:border-accent/60 text-rule hover:text-accent transition-colors" data-region="goto" data-url="${escapeHtml(navigateUrl)}">Go →</button>`
        : ""}
    </div>
  `;

  row.querySelector('[data-region="copy-btn"]')?.addEventListener("click", () => {
    void navigator.clipboard.writeText(p.peer_id).then(() => {
      const btn = row.querySelector('[data-region="copy-btn"]') as HTMLElement;
      btn.innerHTML = `<svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" class="text-accent"><polyline points="20 6 9 17 4 12"/></svg>`;
      setTimeout(() => {
        btn.innerHTML = `<svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="9" width="13" height="13" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg>`;
      }, 1500);
    });
  });

  row.querySelector('[data-region="goto"]')?.addEventListener("click", () => {
    const url = (row.querySelector('[data-region="goto"]') as HTMLElement).dataset.url!;
    navigate({ view: "robot", url });
  });

  return row;
}

function formatInCluster(p: Participant): string {
  if (p.clusterJoinedEstMs == null) {
    return `<span class="text-rule/65 italic">alone</span>`;
  }
  const ageMs = Math.max(0, Date.now() - p.clusterJoinedEstMs);
  return `<span class="text-accent/90">${escapeHtml(formatAge(ageMs))}</span>`;
}

function formatInSession(p: Participant): string {
  const ageMs = Math.max(0, Date.now() - p.sessionStartedEstMs);
  return escapeHtml(formatAge(ageMs));
}
