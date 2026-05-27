// Top bar — shared chrome across every view. Mounted once at app boot
// (so route changes don't unmount it and the View Transitions API only
// has to crossfade the view body). Always shows the Auki horizontal
// lockup on the far left, a breadcrumb that walks the current route
// (Dashboard › Robot name), then quick-search trigger + gear.
//
// On the robot route the robot name in the breadcrumb is a button that
// opens an identity popover (URL, app, source). The breadcrumb itself
// is just names — load-bearing live state lives on the tiles, not in
// the chrome.

import { escapeHtml } from "../util/escape";
import type { Route } from "../data/router";
import { navigate } from "../data/router";
import type { Daemon } from "../data/daemons";
import {
  subscribeInfo,
  formatAge,
  type InfoSnapshot,
} from "../data/info";
import { shortPeer, subscribeCluster, type ClusterStatus } from "../data/cluster";
import { getDomainName } from "../data/domain";
import { setMic, subscribeMic, type MicSnapshot } from "../data/mic";
import { iconDatabase, iconSearch, iconSettings } from "../icons";
import { showSettingsOverlay } from "./settingsOverlay";
import { openClusterModal } from "../views/clusterModal";
import { openDiscoveryMonitor } from "../views/discoveryMonitor";
import { openDomainPromptModal } from "../views/onboarding/domainPromptModal";

// Show the right modifier-key hint based on platform — `navigator.platform`
// is deprecated but still widely populated in Chrome/Safari/Firefox.
const PLATFORM_KEY_HINT =
  typeof navigator !== "undefined" && /Mac/i.test(navigator.platform)
    ? "⌘K"
    : "Ctrl K";

function makeMicToggle(currentPeerId: () => string | null): HTMLButtonElement {
  const btn = document.createElement("button");
  btn.className =
    "h-8 px-3 flex items-center gap-2 text-[12px] rounded-sm transition-colors text-rule hover:text-paper hover:bg-paper/5 border border-paper/10 hover:border-paper/30";
  btn.style.fontFamily = "var(--font-display)";
  // SVG mic icon (Heroicons "microphone" — outline, 16 px). Inline so
  // we don't need a new entry in icons.ts for one consumer.
  const ICON_MIC = `<svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><path d="M12 1.5a3 3 0 0 0-3 3v6a3 3 0 0 0 6 0v-6a3 3 0 0 0-3-3Z"/><path d="M5 11a7 7 0 0 0 14 0M12 18v3M8 21h8"/></svg>`;
  btn.innerHTML = `
    <span class="shrink-0" data-region="mic-icon">${ICON_MIC}</span>
    <span class="text-[10px] uppercase tracking-[0.15em] text-rule/60" data-region="mic-label">Mic off</span>
  `;
  const iconEl = btn.querySelector<HTMLElement>('[data-region="mic-icon"]')!;
  const labelEl = btn.querySelector<HTMLElement>('[data-region="mic-label"]')!;

  let snap: MicSnapshot = {
    enabled: false,
    sensorId: "",
    refreshedAtMs: 0,
  };
  let busy = false;

  const render = () => {
    if (snap.enabled) {
      btn.classList.add(
        "text-accent",
        "hover:text-accent",
        "border-accent/50",
        "hover:border-accent",
        "bg-accent/10",
      );
      btn.classList.remove(
        "text-rule",
        "hover:text-paper",
        "border-paper/10",
        "hover:border-paper/30",
        "hover:bg-paper/5",
      );
      iconEl.style.color = "var(--color-accent, #f97316)";
      labelEl.textContent = "Mic on";
      btn.title = snap.sensorId
        ? `Mic capturing — broadcasting on ${snap.sensorId}`
        : "Mic capturing";
    } else {
      btn.classList.remove(
        "text-accent",
        "hover:text-accent",
        "border-accent/50",
        "hover:border-accent",
        "bg-accent/10",
      );
      btn.classList.add(
        "text-rule",
        "hover:text-paper",
        "border-paper/10",
        "hover:border-paper/30",
        "hover:bg-paper/5",
      );
      iconEl.style.color = "";
      labelEl.textContent = "Mic off";
      btn.title = "Operator mic — click to broadcast to subscribed K1s";
    }
  };

  btn.addEventListener("click", () => {
    if (busy) return;
    busy = true;
    const target = !snap.enabled;
    void setMic(target, currentPeerId())
      .catch((e: unknown) => {
        const msg = e instanceof Error ? e.message : String(e);
        console.warn("mic toggle failed:", msg);
        // Brief red flash on the border so the operator sees the
        // failure even if they're not looking at the console.
        btn.classList.add("border-rose-500/70");
        setTimeout(() => btn.classList.remove("border-rose-500/70"), 1200);
        btn.title = `Mic toggle failed: ${msg}`;
      })
      .finally(() => {
        busy = false;
      });
  });

  subscribeMic((next) => {
    snap = next;
    render();
  });
  render();

  return btn;
}

function makeQuickSearchTrigger(onOpen: () => void): HTMLButtonElement {
  const btn = document.createElement("button");
  btn.className =
    "flex items-center gap-2 pl-2.5 pr-2 py-1 bg-ink-alt border border-paper/10 hover:border-paper/30 rounded-sm text-rule hover:text-paper text-xs transition-colors min-w-0";
  btn.title = "Quick search";
  btn.innerHTML = `
    <span class="shrink-0">${iconSearch(14)}</span>
    <span class="hidden lg:inline">Search…</span>
    <span class="hidden lg:inline-flex items-center gap-0.5 ml-2 text-[11px] text-rule/60">
      <kbd class="px-1 py-0.5 border border-paper/10 rounded text-[10px] font-mono">${PLATFORM_KEY_HINT}</kbd>
    </span>
  `;
  btn.addEventListener("click", () => onOpen());
  return btn;
}

export type TopbarHandle = {
  el: HTMLElement;
  update(route: Route, daemon: Daemon | undefined): void;
};

export function mountTopbar(onOpenQuickSearch: () => void): TopbarHandle {
  const el = document.createElement("header");
  el.className =
    "h-14 px-5 flex items-center justify-between border-b border-paper/10 bg-ink shrink-0 gap-4";

  // ─── left: brand lockup + breadcrumb ─────────────────────────────────
  const left = document.createElement("div");
  left.className = "flex items-center gap-3 min-w-0";

  const home = document.createElement("button");
  home.className =
    "flex items-center gap-2 shrink-0 hover:opacity-80 transition-opacity";
  home.title = "Park · home";
  home.innerHTML = `
    <img src="/brand/auki-monogram-white.svg" alt="" class="h-7 w-auto" />
    <img src="/brand/auki-wordmark-white.svg" alt="Auki" class="h-[11px] w-auto" />
  `;
  home.addEventListener("click", () => navigate({ view: "directory" }));
  left.appendChild(home);

  const sep = document.createElement("span");
  sep.className = "text-paper/30 shrink-0";
  sep.textContent = "·";
  left.appendChild(sep);

  const trail = document.createElement("nav");
  trail.className = "flex items-center gap-2 min-w-0 text-sm";
  trail.setAttribute("aria-label", "breadcrumb");
  left.appendChild(trail);

  // ─── right: domain chip + cluster nav + quick search + settings ──────
  const right = document.createElement("div");
  right.className = "flex items-center gap-2 shrink-0";

  // Domain chip. Shows the saved Domain name (server-reported once
  // `/api/cluster/status` resolves; falls back to the localStorage
  // cache during the brief window between modal submit and the next
  // poll). Clicking opens the prompt in reopen mode so the operator
  // can edit. Hidden until a Domain is known — the first-boot modal
  // handles the "no Domain anywhere" case.
  const domainChip = document.createElement("button");
  domainChip.className =
    "h-8 px-3 flex items-center gap-2 text-[12px] rounded-sm transition-colors text-rule hover:text-paper hover:bg-paper/5 border border-paper/10 hover:border-paper/30";
  domainChip.style.fontFamily = "var(--font-display)";
  domainChip.title = "Domain — click to edit";
  domainChip.innerHTML = `
    <span class="text-[10px] uppercase tracking-[0.15em] text-rule/60">Domain</span>
    <span class="font-mono text-paper/90 truncate max-w-[260px]" data-region="domain-name">—</span>
  `;
  const domainNameEl = domainChip.querySelector<HTMLElement>('[data-region="domain-name"]')!;
  let lastStatus: ClusterStatus | null = null;
  const refreshDomainChip = () => {
    let label: string | null = null;
    if (lastStatus?.source.kind === "in_cluster") {
      label = lastStatus.source.cluster_name;
    }
    if (!label) {
      label = getDomainName();
    }
    if (label) {
      domainNameEl.textContent = label;
      domainNameEl.title = label;
      domainChip.style.display = "";
    } else {
      domainChip.style.display = "none";
    }
  };
  domainChip.addEventListener("click", () => {
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
  const unsubChipCluster = subscribeCluster((snap) => {
    lastStatus = snap.status;
    refreshDomainChip();
  });
  window.addEventListener("auki:domain-changed", refreshDomainChip);
  window.addEventListener("storage", refreshDomainChip);
  refreshDomainChip();
  // The topbar is mounted once for the lifetime of the app; the unsub
  // is held so a future tear-down path (theming experiments, hot
  // reload) can clean up. Today nothing triggers it intentionally.
  void unsubChipCluster;
  right.appendChild(domainChip);

  const discoveryLink = document.createElement("button");
  discoveryLink.className =
    "h-8 px-3 flex items-center gap-2 text-[12px] uppercase rounded-sm transition-colors text-rule hover:text-paper hover:bg-paper/5";
  discoveryLink.style.fontFamily = "var(--font-display)";
  discoveryLink.title = "Discovery monitor";
  discoveryLink.innerHTML = `
    <span class="shrink-0">${iconDatabase(14)}</span>
    <span class="hidden xl:inline">Discovery</span>
  `;
  discoveryLink.addEventListener("click", () => openDiscoveryMonitor());
  right.appendChild(discoveryLink);

  const clusterLink = document.createElement("button");
  clusterLink.className =
    "h-8 px-3 flex items-center gap-2 text-[12px] uppercase tracking-[0.15em] rounded-sm transition-colors";
  clusterLink.style.fontFamily = "var(--font-display)";
  clusterLink.title = "Cluster participants (libp2p peers)";
  clusterLink.innerHTML = `
    <span class="w-1.5 h-1.5 rounded-full bg-accent shrink-0"></span>
    <span data-region="cluster-label">Cluster</span>
  `;
  clusterLink.addEventListener("click", () => openClusterModal());
  right.appendChild(clusterLink);

  // Mic toggle — operator microphone broadcast to the robot the
  // operator is currently inspecting. Hidden off-route (dashboard,
  // session viewer, cluster modal) because the backend refuses
  // mic-on without inspect focus and the audio sensor is hidden
  // from the catalog when no robot is focused. Click toggles
  // capture on/off; the pill flashes red briefly on backend
  // rejection.
  let currentMicPeerId: string | null = null;
  const micToggle = makeMicToggle(() => currentMicPeerId);
  // Hidden until the first update() call resolves the route — avoids
  // a flash of the toggle on the dashboard during boot.
  micToggle.style.display = "none";
  right.appendChild(micToggle);

  right.appendChild(makeQuickSearchTrigger(onOpenQuickSearch));

  const settings = document.createElement("button");
  settings.className =
    "w-8 h-8 flex items-center justify-center text-rule hover:text-paper rounded-sm transition-colors";
  settings.title = "Settings";
  settings.innerHTML = iconSettings(16);
  settings.addEventListener("click", () => showSettingsOverlay());
  right.appendChild(settings);

  el.appendChild(left);
  el.appendChild(right);

  // ─── update: rebuild the breadcrumb ──────────────────────────────────
  // Cluster button styling is static — it opens a modal, not a route.
  clusterLink.className =
    "h-8 px-3 flex items-center gap-2 text-[12px] uppercase tracking-[0.15em] rounded-sm transition-colors text-rule hover:text-paper hover:bg-paper/5";

  const update = (route: Route, daemon: Daemon | undefined) => {
    currentMicPeerId = route.view === "robot" ? route.url : null;
    micToggle.style.display = route.view === "robot" ? "" : "none";

    const crumbs = breadcrumbFor(route, daemon);
    trail.replaceChildren();
    crumbs.forEach((c, i) => {
      if (i > 0) {
        const caret = document.createElement("span");
        caret.className = "text-paper/30 shrink-0";
        caret.textContent = "›";
        trail.appendChild(caret);
      }
      const isLast = i === crumbs.length - 1;
      if (c.route && !isLast) {
        const link = document.createElement("button");
        link.className =
          "text-paper/60 hover:text-paper transition-colors truncate max-w-[200px]";
        link.textContent = c.label;
        link.addEventListener("click", () => navigate(c.route!));
        trail.appendChild(link);
      } else if (isLast && route.view === "robot" && daemon) {
        // Robot crumb: clickable name that opens identity popover.
        const wrap = document.createElement("span");
        wrap.className = "relative flex items-center";
        const btn = document.createElement("button");
        btn.className =
          "text-paper truncate max-w-[260px] hover:text-accent transition-colors";
        btn.textContent = c.label;
        btn.title = "Show device details";
        btn.addEventListener("click", () => openIdentityPopover(btn, daemon));
        wrap.appendChild(btn);
        trail.appendChild(wrap);
      } else {
        const span = document.createElement("span");
        span.className = isLast
          ? "text-paper truncate max-w-[260px]"
          : "text-paper/60 truncate max-w-[200px]";
        span.textContent = c.label;
        trail.appendChild(span);
      }
    });
  };

  return { el, update };
}

type Crumb = { label: string; route?: Route };

function breadcrumbFor(route: Route, daemon: Daemon | undefined): Crumb[] {
  const trail: Crumb[] = [{ label: "Dashboard", route: { view: "directory" } }];
  if (route.view === "robot") {
    trail.push({ label: daemon ? daemon.name : "device" });
  } else if (route.view === "cluster") {
    trail.push({ label: "Cluster" });
  }
  return trail;
}

// ─── identity popover ────────────────────────────────────────────────────────

let openPopover: { el: HTMLElement; close: () => void } | null = null;

function openIdentityPopover(anchor: HTMLElement, daemon: Daemon): void {
  // Same anchor click while open → close. Different anchor → swap.
  if (openPopover) {
    const wasOpen = openPopover;
    wasOpen.close();
    openPopover = null;
    return;
  }
  const rect = anchor.getBoundingClientRect();
  const pop = document.createElement("div");
  pop.className =
    "fixed z-50 bg-ink-alt border border-paper/15 rounded-md shadow-2xl px-4 py-3 min-w-[300px] text-xs";
  pop.style.top = `${rect.bottom + 6}px`;
  pop.style.left = `${rect.left}px`;
  document.body.appendChild(pop);

  let snapshot: InfoSnapshot | null = null;

  // Render the popover. Called on mount, on info-snapshot updates, and
  // every second by the live-age timer so session-age stays current.
  const render = () => {
    const ageStr = snapshot
      ? formatAge(Date.now() - snapshot.sessionStartedEstMs)
      : null;
    const clusterStr = clusterLabel(snapshot);
    pop.innerHTML = `
      <div class="font-medium text-paper text-sm mb-2.5" style="font-family: var(--font-display)">
        ${escapeHtml(daemon.name)}
        ${ageStr ? `<span class="text-rule/70 text-[11px] font-normal ml-1.5" style="font-family: var(--font-sans, inherit)">· running ${escapeHtml(ageStr)}</span>` : ""}
      </div>
      <dl class="space-y-1.5">
        ${row("App", escapeHtml(daemon.app))}
        ${row("Source", escapeHtml(daemon.source), "non-mono")}
        ${
          snapshot
            ? row("Cluster", clusterStr)
            : ""
        }
        ${
          snapshot
            ? row(
                "Peer",
                `<span class="font-mono text-[11px]" title="${escapeHtml(snapshot.info.peer_id)}">${escapeHtml(shortPeer(snapshot.info.peer_id))}</span>`,
                "raw",
              )
            : ""
        }
        ${
          snapshot
            ? row(
                "Machine",
                `<span class="font-mono text-[11px]" title="app_instance — first non-loopback MAC, lowercased hex">${escapeHtml(snapshot.info.app_instance)}</span>`,
                "raw",
              )
            : ""
        }
        ${row(
          "URL",
          `<span class="font-mono text-[11px]" title="${escapeHtml(daemon.url)}">${escapeHtml(daemon.url)}</span>`,
          "raw",
        )}
        ${
          !snapshot
            ? `<div class="text-rule/55 text-[11px] italic pt-1.5 border-t border-paper/10 mt-1.5">extra identity fields require SDK v0.0.12+</div>`
            : ""
        }
      </dl>
    `;
  };

  render();

  // Live age tick — once a second, re-render so "running 12s" advances.
  const ageTimer = window.setInterval(() => {
    if (snapshot) render();
  }, 1000);

  // /api/info subscription — fills in peer/app_instance/cluster the
  // moment the daemon answers. A daemon that fails to answer leaves
  // the popover in basic mode (peer-id + placeholder app); the
  // directory card already wears a `no /api/info` chip for the same
  // daemon, so duplicating the warning here would be noise.
  const unsubInfo = subscribeInfo(daemon.url, (snap) => {
    snapshot = snap;
    render();
  });

  const close = () => {
    pop.remove();
    document.removeEventListener("click", onOutside, true);
    clearInterval(ageTimer);
    unsubInfo();
    openPopover = null;
  };

  const onOutside = (e: Event) => {
    if (!(e.target instanceof Node)) return;
    if (pop.contains(e.target) || anchor.contains(e.target)) return;
    close();
  };

  setTimeout(() => document.addEventListener("click", onOutside, true), 0);
  openPopover = { el: pop, close };
}

function row(label: string, value: string, mode: "default" | "non-mono" | "raw" = "default"): string {
  const dd =
    mode === "raw"
      ? `<dd class="text-paper/80 truncate text-right">${value}</dd>`
      : `<dd class="text-paper/80 truncate text-right">${value}</dd>`;
  return `
    <div class="flex items-baseline justify-between gap-3">
      <dt class="text-rule/70 uppercase tracking-[0.12em] text-[10px] shrink-0">${escapeHtml(label)}</dt>
      ${dd}
    </div>
  `;
}

function clusterLabel(snap: InfoSnapshot | null): string {
  if (!snap) return "—";
  if (snap.info.cluster_joined_at_ns == null || snap.clusterJoinedEstMs == null) {
    return `<span class="text-rule/70">alone</span>`;
  }
  const inClusterFor = formatAge(Date.now() - snap.clusterJoinedEstMs);
  return `<span class="text-accent/90">in cluster · ${escapeHtml(inClusterFor)}</span>`;
}
