// Park-self card — Park's own row in the dashboard's Online list.
//
// Park IS a peer in the cluster, so its identity gets a card too. Same
// shell as `makeLiveCard` so it reads as a sibling of the other peers.
// Distinguishing affordances:
//
//   • A muted "you · Park" prefix on the identity line.
//   • The Domain identity (canonical `{wallet_id}/{name}`, or the
//     SDK's reserved singleton string) shown in the body so the
//     operator sees what Park introduces itself as to joiners.
//   • Click → opens the domain-prompt modal so the operator can switch
//     domains from the dashboard.
//
// `subscribeCluster` polls `/api/cluster/status` on a 1s cadence; the
// localStorage fallback covers the ~1s gap between modal submit and
// the next poll.

import { iconRobot } from "../../icons";
import {
  subscribeCluster,
  shortPeer,
  type Participant,
  type ClusterStatus,
} from "../../data/cluster";
import { getDomainName } from "../../data/domain";
import { subscribeMic, type MicSnapshot } from "../../data/mic";
import { openDomainPromptModal } from "../onboarding/domainPromptModal";
import type { CardHandle } from "./robotCard";

export function makeParkSelfCard(): CardHandle {
  // Build the same card shell as other Online cards (kept in sync with
  // robotCard.ts's `makeCardShell`). Park's card is a button so the
  // whole tile is clickable.
  const el = document.createElement("button");
  el.className =
    "group text-left bg-ink-alt border rounded-md overflow-hidden flex flex-col border-accent/40 hover:border-accent/70 transition-colors";

  el.innerHTML = `
    <div class="relative w-full aspect-video bg-ink overflow-hidden" data-region="thumb">
      <div class="absolute inset-0 flex items-center justify-center text-accent/70" data-region="icon">${iconRobot(48)}</div>
      <div class="absolute top-2 left-2 px-2 py-0.5 rounded-sm bg-accent/20 border border-accent/50 text-paper text-[10px] uppercase tracking-[0.15em]" style="font-family: var(--font-display)" data-region="self-pill">you · Park</div>
      <div class="absolute top-2 right-2 px-2 py-0.5 rounded-sm bg-paper/10 border border-paper/30 text-paper text-[10px] uppercase tracking-[0.15em] hidden" style="font-family: var(--font-display)" data-region="manager-pill">Manager</div>
    </div>
    <div class="px-3 pt-2.5 pb-3">
      <div class="text-paper text-sm font-medium truncate mb-0.5" data-region="name">Park</div>
      <div class="text-rule text-xs mb-2" data-region="app">park</div>
      <div class="flex items-center justify-between text-[12px] mb-2">
        <span class="text-paper/70" data-region="state">— sensors</span>
        <span class="text-rule" data-region="recording">idle</span>
      </div>
      <div class="flex items-center justify-between text-[12px]">
        <span class="text-paper/70" data-region="domain-line">no domain set</span>
        <span class="text-rule" data-region="peer-id">—</span>
      </div>
    </div>
  `;

  const managerPill = el.querySelector<HTMLElement>('[data-region="manager-pill"]')!;
  const domainLine = el.querySelector<HTMLElement>('[data-region="domain-line"]')!;
  const peerIdEl = el.querySelector<HTMLElement>('[data-region="peer-id"]')!;
  const stateEl = el.querySelector<HTMLElement>('[data-region="state"]')!;
  const recordingEl = el.querySelector<HTMLElement>('[data-region="recording"]')!;

  // Click anywhere on the card → open the domain prompt in reopen mode.
  el.addEventListener("click", () => {
    const initialName =
      lastStatus?.source.kind === "in_cluster"
        ? lastStatus.source.cluster_name
        : getDomainName() ?? "";
    const initialDiscoveryUrl =
      lastStatus?.source.kind === "in_cluster"
        ? lastStatus.source.url
        : lastStatus?.discovery_url ?? "";
    openDomainPromptModal({ initialName, initialDiscoveryUrl });
  });

  let lastStatus: ClusterStatus | null = null;
  let micSnap: MicSnapshot | null = null;

  // Paint the domain line. Server-reported status is the source of
  // truth; localStorage covers the ~1s window between modal submit
  // and the next cluster poll.
  const repaint = () => {
    let domainText: string | null = null;
    let showManager = false;
    if (lastStatus?.source.kind === "in_cluster") {
      domainText = lastStatus.source.cluster_name;
      showManager = lastStatus.source.is_manager;
    } else {
      const cached = getDomainName();
      if (cached) domainText = cached;
    }

    if (domainText) {
      domainLine.textContent = `domain · ${domainText}`;
      domainLine.className = "text-paper/85 truncate";
      domainLine.title = domainText;
    } else {
      domainLine.textContent = "no domain set — click to name one";
      domainLine.className = "text-rule/60 italic";
      domainLine.title = "";
    }

    managerPill.classList.toggle("hidden", !showManager);
  };

  const repaintMic = () => {
    const sensorCount = micSnap?.sensorId ? 1 : 0;
    stateEl.textContent = `${sensorCount} sensor${sensorCount === 1 ? "" : "s"}`;
    if (micSnap?.enabled) {
      recordingEl.innerHTML = `<span class="inline-flex items-center gap-1 text-accent"><span class="w-1.5 h-1.5 rounded-full bg-accent animate-pulse"></span>mic on</span>`;
    } else {
      recordingEl.textContent = "idle";
    }
  };

  const applyClusterSelf = (self: Participant | null) => {
    if (self && self.peer_id) {
      peerIdEl.textContent = shortPeer(self.peer_id);
      peerIdEl.title = self.peer_id;
    } else {
      peerIdEl.textContent = "—";
      peerIdEl.title = "Park's /api/info hasn't resolved yet";
    }
  };

  const unsubCluster = subscribeCluster((snap) => {
    lastStatus = snap.status;
    applyClusterSelf(snap.self);
    repaint();
  });

  const unsubMic = subscribeMic((snap) => {
    micSnap = snap;
    repaintMic();
  });

  // Listen for `auki:domain-changed` so the card refreshes from
  // localStorage immediately on modal submit (cluster poll catches up
  // ~1s later). Native `storage` event picks up cross-tab edits.
  const onDomainChanged = () => repaint();
  window.addEventListener("auki:domain-changed", onDomainChanged);
  window.addEventListener("storage", onDomainChanged);

  repaint();
  repaintMic();

  return {
    el,
    dispose() {
      unsubCluster();
      unsubMic();
      window.removeEventListener("auki:domain-changed", onDomainChanged);
      window.removeEventListener("storage", onDomainChanged);
    },
  };
}
