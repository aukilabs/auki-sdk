// Domain-prompt modal — Park's startup gate.
//
// What this is
// ────────────
// Park boots with no cluster registration. The operator types:
//   1. A Discovery service URL
//   2. A Domain name
// Park calls Discovery's idempotent `register` — if the Domain already
// exists, Park joins it; if not, Park creates it with itself as the
// sole peer. Both outcomes produce `ClusterSource::InCluster` on the
// Rust side and unblock the libp2p stream paths.
//
// The modal is MANDATORY when Park is in `not_in_cluster` — Esc, close
// button, and overlay click are all disabled. It can be re-opened
// later (from the topbar chip) to switch Domains; in that mode the
// dismiss affordances are restored.
//
// localStorage caches the last accepted canonical name purely as a
// warmup hint for the topbar chip — the cluster poller is the source
// of truth.

import {
  validate,
  reasonLabel,
  DOMAIN_NAME_MAX,
} from "../../data/domainName";
import { setDomainName } from "../../data/domain";
import { showSettingsOverlay } from "../../shell/settingsOverlay";
import { sdkRuntime } from "../../sdk/runtime";

let openInstance = false;

export type DomainPromptOpts = {
  /** Initial Domain name to seed the input with. Empty for first boot. */
  initialName?: string;
  /** Initial Discovery URL to seed the input with. Empty for first boot. */
  initialDiscoveryUrl?: string;
  /** When true, dismissal affordances (Esc, close, Cancel) are
   * disabled — used at boot when Park has no cluster. When false,
   * operator can dismiss to keep Park's current registration. */
  mandatory?: boolean;
};

export function openDomainPromptModal(opts: DomainPromptOpts = {}): void {
  if (openInstance) return;
  openInstance = true;

  const mandatory = opts.mandatory ?? false;

  const overlay = document.createElement("div");
  overlay.className =
    "fixed inset-0 z-[55] flex items-center justify-center bg-ink/85 backdrop-blur-sm px-4";

  const panel = document.createElement("div");
  panel.className =
    "relative w-full max-w-md rounded-md border border-paper/15 bg-ink-alt shadow-2xl p-6";
  overlay.appendChild(panel);

  const closeBtn = document.createElement("button");
  closeBtn.className =
    "absolute top-3 right-3 w-7 h-7 flex items-center justify-center rounded-sm hover:bg-paper/8 text-rule/70 hover:text-paper transition-colors text-base leading-none";
  closeBtn.title = "Close (Esc)";
  closeBtn.setAttribute("aria-label", "Close");
  closeBtn.textContent = "×";
  closeBtn.addEventListener("click", () => {
    if (!mandatory) close();
  });
  if (mandatory) closeBtn.style.display = "none";
  panel.appendChild(closeBtn);

  panel.innerHTML += `
    <div class="text-accent text-[11px] tracking-[0.3em] uppercase mb-2" style="font-family: var(--font-wordmark)">Park · domain</div>
    <div class="text-paper text-xl font-light leading-tight mb-1" style="font-family: var(--font-display)">Join or create a domain</div>
    <div class="text-rule/80 text-[12px] leading-snug mb-5">
      Park registers against Discovery and joins the domain you name.
      If no domain with that name exists yet, Park creates it.
    </div>

    <label class="block text-rule text-[11px] uppercase tracking-[0.2em] mb-1.5" style="font-family: var(--font-display)" for="discovery-url-input">Discovery URL</label>
    <input
      id="discovery-url-input"
      type="text"
      autocomplete="off"
      autocapitalize="none"
      autocorrect="off"
      spellcheck="false"
      class="w-full px-3 py-2 bg-ink border border-paper/15 focus:border-accent rounded-sm text-paper text-sm outline-none transition-colors"
      placeholder="http://192.168.9.10:8080" />

    <div class="mt-4">
      <div class="flex items-center justify-between mb-1.5">
        <label class="text-rule text-[11px] uppercase tracking-[0.2em]" style="font-family: var(--font-display)">Running clusters</label>
        <span class="text-rule/60 text-[10px]" data-region="cluster-list-status">enter Discovery URL above</span>
      </div>
      <div class="max-h-32 overflow-y-auto rounded-sm border border-paper/10 bg-ink/60" data-region="cluster-list">
        <div class="px-3 py-2 text-rule/50 text-[12px] italic">—</div>
      </div>
    </div>

    <label class="block text-rule text-[11px] uppercase tracking-[0.2em] mb-1.5 mt-4" style="font-family: var(--font-display)" for="domain-name-input">Domain name</label>
    <input
      id="domain-name-input"
      type="text"
      autocomplete="off"
      autocapitalize="none"
      autocorrect="off"
      spellcheck="false"
      maxlength="${DOMAIN_NAME_MAX * 4}"
      class="w-full px-3 py-2 bg-ink border border-paper/15 focus:border-accent rounded-sm text-paper text-sm outline-none transition-colors"
      placeholder="e.g. Atlanta Warehouse" />

    <div class="mt-3 text-[11px] text-rule/70 leading-snug">
      Canonical form
      <span class="font-mono text-paper/90 ml-1" data-region="canonical">—</span>
    </div>

    <div class="mt-3 text-[11px] text-red-300/85 leading-snug min-h-[14px]" data-region="error"></div>

    <div class="mt-6 flex items-center justify-between gap-2">
      <div class="flex items-center gap-2">
        <button class="px-4 py-1.5 rounded-sm border border-red-300/40 hover:border-red-300/70 text-red-200/85 hover:text-red-100 text-[12px] transition-colors" data-region="leave" ${mandatory ? 'style="display: none"' : ""}>Leave cluster</button>
        <button class="px-4 py-1.5 rounded-sm border border-paper/15 hover:border-paper/30 text-rule hover:text-paper text-[12px] transition-colors" data-region="settings">Settings</button>
      </div>
      <div class="flex items-center gap-2">
        <button class="px-4 py-1.5 rounded-sm border border-paper/15 hover:border-paper/30 text-rule hover:text-paper text-[12px] transition-colors" data-region="cancel" ${mandatory ? 'style="display: none"' : ""}>Cancel</button>
        <button class="px-4 py-1.5 rounded-sm border border-accent/50 hover:bg-accent/15 text-paper text-[12px] transition-colors disabled:opacity-40 disabled:cursor-not-allowed" data-region="join" disabled>Join existing</button>
        <button class="px-4 py-1.5 rounded-sm bg-accent/20 hover:bg-accent/30 border border-accent/50 text-paper text-[12px] transition-colors disabled:opacity-40 disabled:cursor-not-allowed" data-region="submit" disabled>Create new</button>
      </div>
    </div>
  `;

  let currentCanonical = "";
  let currentName = "";
  let currentNameOk = false;

  const urlInput = panel.querySelector<HTMLInputElement>("#discovery-url-input")!;
  const nameInput = panel.querySelector<HTMLInputElement>("#domain-name-input")!;
  const canonicalRegion = panel.querySelector<HTMLElement>('[data-region="canonical"]')!;
  const errorRegion = panel.querySelector<HTMLElement>('[data-region="error"]')!;
  const submitBtn = panel.querySelector<HTMLButtonElement>('[data-region="submit"]')!;
  const joinBtn = panel.querySelector<HTMLButtonElement>('[data-region="join"]')!;
  const cancelBtn = panel.querySelector<HTMLButtonElement>('[data-region="cancel"]')!;
  const leaveBtn = panel.querySelector<HTMLButtonElement>('[data-region="leave"]')!;
  const settingsBtn = panel.querySelector<HTMLButtonElement>('[data-region="settings"]')!;
  const clusterList = panel.querySelector<HTMLElement>('[data-region="cluster-list"]')!;
  const clusterListStatus = panel.querySelector<HTMLElement>('[data-region="cluster-list-status"]')!;

  // ─── Discovery cluster directory ──────────────────────────────────
  // Auto-fetch when the URL field has a usable value. Refreshes
  // periodically while the modal is open so peer-count + freshly-
  // created clusters appear without operator intervention. Clicking
  // an entry fills the Domain name input.
  type ClusterEntry = { name: string; peer_count: number; manager_peer_id: string };
  let clusters: ClusterEntry[] = [];
  let lastFetchedUrl = "";
  let fetchToken = 0;

  const renderClusters = () => {
    if (clusters.length === 0) {
      clusterList.innerHTML = `<div class="px-3 py-2 text-rule/50 text-[12px] italic">no clusters in this Discovery yet</div>`;
      return;
    }
    clusterList.innerHTML = clusters
      .map(
        (c) => `
          <button data-cluster-name="${escapeAttr(c.name)}" class="w-full flex items-center justify-between px-3 py-2 text-left hover:bg-paper/8 border-b border-paper/5 last:border-b-0 transition-colors">
            <span class="text-paper text-[13px] font-mono truncate" title="${escapeAttr(c.name)}">${escapeText(c.name)}</span>
            <span class="text-rule/70 text-[11px] shrink-0 ml-3">${c.peer_count} ${c.peer_count === 1 ? "peer" : "peers"}</span>
          </button>`,
      )
      .join("");
    clusterList.querySelectorAll<HTMLButtonElement>("button[data-cluster-name]").forEach((btn) => {
      btn.addEventListener("click", () => {
        const n = btn.dataset.clusterName ?? "";
        if (n.length === 0) return;
        nameInput.value = n;
        refresh();
        nameInput.focus();
      });
    });
  };

  const fetchClusters = async () => {
    const url = urlInput.value.trim();
    if (!isLikelyUrl(url)) {
      clusterListStatus.textContent = "enter Discovery URL above";
      clusters = [];
      clusterList.innerHTML = `<div class="px-3 py-2 text-rule/50 text-[12px] italic">—</div>`;
      lastFetchedUrl = "";
      return;
    }
    const token = ++fetchToken;
    clusterListStatus.textContent = lastFetchedUrl === url ? "updating…" : "loading…";
    try {
      const body = await sdkRuntime.listClusters(url);
      if (token !== fetchToken) return; // a newer request superseded us
      clusters = body;
      lastFetchedUrl = url;
      clusterListStatus.textContent = `${clusters.length} cluster${clusters.length === 1 ? "" : "s"}`;
      renderClusters();
    } catch (e) {
      if (token !== fetchToken) return;
      const msg = e instanceof Error ? e.message : String(e);
      clusterListStatus.textContent = "unreachable";
      clusters = [];
      clusterList.innerHTML = `<div class="px-3 py-2 text-red-300/80 text-[12px]">${escapeText(msg)}</div>`;
    }
  };

  // Debounced fetch on URL edits; long-running refresh while modal is
  // open so freshly-created clusters appear without operator action.
  let urlDebounceTimer: number | null = null;
  const scheduleFetch = () => {
    if (urlDebounceTimer !== null) window.clearTimeout(urlDebounceTimer);
    urlDebounceTimer = window.setTimeout(() => {
      urlDebounceTimer = null;
      void fetchClusters();
    }, 300);
  };
  urlInput.addEventListener("input", scheduleFetch);

  const refreshTimer = window.setInterval(() => void fetchClusters(), 2000);

  const refresh = () => {
    const rawName = nameInput.value;
    const rawUrl = urlInput.value.trim();
    const v = validate(rawName);
    canonicalRegion.textContent = v.canonical.length > 0 ? v.canonical : "—";
    currentCanonical = v.canonical;
    currentName = rawName.trim();

    const urlOk = isLikelyUrl(rawUrl);
    currentNameOk = v.ok;
    submitBtn.disabled = !(urlOk && v.ok);
    joinBtn.disabled = !(urlOk && v.ok);

    if (rawName.length === 0 || v.ok) {
      errorRegion.textContent = "";
    } else {
      errorRegion.textContent = reasonLabel(v.reason);
    }
  };

  urlInput.addEventListener("input", refresh);
  nameInput.addEventListener("input", refresh);

  if (opts.initialDiscoveryUrl) urlInput.value = opts.initialDiscoveryUrl;
  if (opts.initialName) nameInput.value = opts.initialName;
  refresh();

  let inflight = false;
  const submit = async (endpoint: "create" | "join") => {
    if (!currentNameOk || inflight) return;
    const url = urlInput.value.trim();
    if (!isLikelyUrl(url)) return;
    inflight = true;
    submitBtn.disabled = true;
    joinBtn.disabled = true;
    const verb = endpoint === "create" ? "Creating…" : "Joining…";
    const activeBtn = endpoint === "create" ? submitBtn : joinBtn;
    const originalLabel = activeBtn.textContent;
    activeBtn.textContent = verb;
    errorRegion.textContent = "";
    try {
      const body = await sdkRuntime.enterCluster({
        discoveryUrl: url,
        clusterName: currentName.length > 0 ? currentName : currentCanonical,
        mode: endpoint,
      });
      if (body.source.kind === "in_cluster") {
        setDomainName(body.source.cluster_name);
      }
      window.dispatchEvent(new CustomEvent("auki:domain-changed", { detail: body }));
      close();
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      errorRegion.textContent = msg;
    } finally {
      inflight = false;
      submitBtn.disabled = !currentNameOk;
      joinBtn.disabled = !currentNameOk;
      if (originalLabel !== null) activeBtn.textContent = originalLabel;
    }
  };

  submitBtn.addEventListener("click", () => {
    void submit("create");
  });
  joinBtn.addEventListener("click", () => {
    void submit("join");
  });

  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Enter") {
      e.preventDefault();
      // Default Enter action = Join (the common case once a cluster
      // exists). Operator clicks "Create new" explicitly the first
      // time. Could be flipped via a preference if a different default
      // is more demo-friendly.
      void submit("join");
    } else if (e.key === "Escape") {
      if (!mandatory) {
        e.preventDefault();
        close();
      }
    }
  };
  urlInput.addEventListener("keydown", onKey);
  nameInput.addEventListener("keydown", onKey);

  cancelBtn.addEventListener("click", () => {
    if (!mandatory) close();
  });
  settingsBtn.addEventListener("click", () => showSettingsOverlay());

  leaveBtn.addEventListener("click", async () => {
    if (mandatory) return;
    if (inflight) return;
    inflight = true;
    leaveBtn.disabled = true;
    const originalLabel = leaveBtn.textContent;
    leaveBtn.textContent = "Leaving…";
    errorRegion.textContent = "";
    try {
      const body = await sdkRuntime.leaveCluster();
      window.dispatchEvent(new CustomEvent("auki:domain-changed", { detail: body }));
      close();
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      errorRegion.textContent = msg;
    } finally {
      inflight = false;
      leaveBtn.disabled = false;
      if (originalLabel !== null) leaveBtn.textContent = originalLabel;
    }
  });

  const onDocKeyDown = (e: KeyboardEvent) => {
    if (mandatory) return;
    if (e.key === "Escape") {
      e.preventDefault();
      close();
    }
  };
  document.addEventListener("keydown", onDocKeyDown);

  function close(): void {
    document.removeEventListener("keydown", onDocKeyDown);
    window.clearInterval(refreshTimer);
    if (urlDebounceTimer !== null) window.clearTimeout(urlDebounceTimer);
    overlay.remove();
    openInstance = false;
  }

  document.body.appendChild(overlay);
  urlInput.focus();
  // Kick off the cluster list immediately if a Discovery URL was
  // pre-seeded (e.g. operator re-opening the modal to switch domains).
  if (isLikelyUrl(urlInput.value.trim())) {
    void fetchClusters();
  }
}

function isLikelyUrl(s: string): boolean {
  if (s.length === 0) return false;
  return s.startsWith("http://") || s.startsWith("https://");
}

function escapeAttr(s: string): string {
  return s.replace(/[&<>"']/g, (c) =>
    c === "&" ? "&amp;" : c === "<" ? "&lt;" : c === ">" ? "&gt;" : c === "\"" ? "&quot;" : "&#39;",
  );
}
function escapeText(s: string): string {
  return s.replace(/[&<>]/g, (c) => (c === "&" ? "&amp;" : c === "<" ? "&lt;" : "&gt;"));
}

export { getDomainName } from "../../data/domain";
