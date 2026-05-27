import "./style.css";
import { onRouteChange, getRoute, type Route } from "./data/router";
import { findDaemon, subscribeDaemons } from "./data/daemons";
import { subscribeCluster } from "./data/cluster";
import { startInspectFocusSync } from "./data/inspect";
import { mountTopbar } from "./shell/topbar";
import { mountQuickSearch } from "./shell/quickSearch";
import { directory } from "./views/directory";
import { openDomainPromptModal } from "./views/onboarding/domainPromptModal";
import { withViewTransition } from "./anim";
import { sdkRuntime, type ClusterSnapshot } from "./sdk/runtime";

// Directory is the landing page — eager so first paint doesn't pay an
// async cost. Robot + cluster are dynamically imported so their
// dependencies (notably ~500 kB of Three.js for the robot view) stay
// out of the initial bundle. Vite splits these into separate chunks
// automatically; subsequent navigations cache the resolved module on
// the lazy* promise so they're instant on second visit.
const lazyRobot = () => import("./views/robot");
const lazyCluster = () => import("./views/cluster");

type View = { el: HTMLElement; dispose: () => void };
type ClusterGate = "unknown" | "not_in_cluster" | "in_cluster";
type DomainChangedDetail = {
  source?: { kind?: string };
};
type OverwatchParkProbe = {
  snapshot: () => ClusterSnapshot;
  sensors: (peerId: string) => unknown[];
  debugState: () => Record<string, unknown>;
  nextSensorFrame: (
    peerId: string,
    sensorId: string,
    maxMessages?: number,
  ) => Promise<unknown>;
};

const app = document.getElementById("app");
if (app) {
  (globalThis as typeof globalThis & { __overwatchPark?: OverwatchParkProbe }).__overwatchPark =
    createProbe();

  app.className = "h-full flex flex-col";

  // Quick-search overlay — mounted once outside #app so route changes
  // don't disturb it. Toggled via ⌘K / Ctrl+K / "/" or the trigger.
  const quickSearch = mountQuickSearch();

  // Persistent chrome: top bar lives at #app's top and is mounted once.
  // Route changes only update the breadcrumb + swap the view body, so
  // View Transitions crossfade just the body and the chrome stays put.
  const topbar = mountTopbar(quickSearch.open);
  app.appendChild(topbar.el);

  const viewMount = document.createElement("div");
  viewMount.className = "flex-1 min-h-0 flex flex-col";
  app.appendChild(viewMount);

  let disposeView: (() => void) | null = null;
  let lastRouteSig = "";
  let swapSeq = 0;
  let clusterGate: ClusterGate = "unknown";

  const render = () => {
    const route = getRoute();
    const daemon =
      clusterGate === "in_cluster" && route.view === "robot"
        ? findDaemon(route.url)
        : undefined;
    const sig = `${clusterGate}:${routeSig(route)}`;

    // Topbar updates immediately — its content shouldn't crossfade,
    // it's persistent chrome.
    topbar.update(route, daemon);

    // Capture a sequence number so that if render() is called again
    // before this swap resolves (e.g. rapid cluster-membership
    // updates), only the last swap wins and earlier ones bail
    // without appending.
    const seq = ++swapSeq;
    const swap = async () => {
      if (disposeView) {
        disposeView();
        disposeView = null;
      }
      viewMount.replaceChildren();
      const view = await mount(route, daemon, clusterGate);
      if (seq !== swapSeq) return;
      viewMount.appendChild(view.el);
      disposeView = view.dispose;
    };

    // Only run a View Transition when the route signature actually
    // changes (avoids a needless crossfade when only the daemon ref
    // updates after the cluster membership poll catches up).
    if (sig !== lastRouteSig) {
      lastRouteSig = sig;
      withViewTransition(swap);
    } else {
      void swap();
    }
  };

  onRouteChange(render);

  // Sync inspect focus to Park's backend so reverse-direction
  // Dialogue audio (K1 mic → speaker) only plays inside the
  // focused K1's robot view. Idempotent; subscribes to the same
  // route-change stream as `render` above.
  startInspectFocusSync();

  // Re-render on daemons updates so the topbar's robot identity catches
  // up once the cluster membership poll resolves the daemon mid-flight
  // on the robot detail view.
  subscribeDaemons(() => {
    if (getRoute().view === "robot") render();
  });

  // Domain prompt. Mandatory at boot — Park starts with no
  // registration, the modal can't be dismissed until the operator
  // supplies a Discovery URL + Domain name. Once the first cluster
  // poll resolves we either close the prompt (Park has joined a
  // Domain via a previous session that's somehow still live) or keep
  // it open and wait.
  let mandatoryOpen = false;
  const unsubscribeCluster = subscribeCluster((snap) => {
    if (!snap.status) return;
    const nextGate: ClusterGate =
      snap.status.source.kind === "not_in_cluster" ? "not_in_cluster" : "in_cluster";
    const gateChanged = clusterGate !== nextGate;
    clusterGate = nextGate;
    if (snap.status.source.kind === "not_in_cluster") {
      if (!mandatoryOpen) {
        mandatoryOpen = true;
        openDomainPromptModal({
          mandatory: true,
          initialDiscoveryUrl: snap.status.discovery_url ?? "",
        });
      }
    } else {
      mandatoryOpen = false;
    }
    if (gateChanged) render();
  });
  // Keep the subscription alive for the lifetime of the app — main.ts
  // never tears down. The disposer is held in scope so a future
  // hot-reload path can call it.
  void unsubscribeCluster;

  // The domain chip in the topbar listens to this event to refresh
  // its label after a submit / clear. main.ts also re-renders so the
  // dashboard's "you are Park" tile picks up the new identity.
  window.addEventListener("auki:domain-changed", (ev) => {
    const nextGate = clusterGateFromDetail((ev as CustomEvent).detail);
    if (nextGate) clusterGate = nextGate;
    render();
  });
}

async function mount(
  route: Route,
  daemon: ReturnType<typeof findDaemon>,
  clusterGate: ClusterGate,
): Promise<View> {
  if (clusterGate !== "in_cluster") return clusterGateView();
  switch (route.view) {
    case "directory":
      return directory();
    case "robot": {
      const m = await lazyRobot();
      return m.robot(daemon);
    }
    case "cluster": {
      const m = await lazyCluster();
      return m.cluster();
    }
  }
}

function clusterGateView(): View {
  const el = document.createElement("main");
  el.className = "flex-1 min-h-0 flex items-center justify-center bg-ink";
  el.innerHTML = `
    <div class="px-6 py-5 text-center">
      <div class="text-accent text-[11px] tracking-[0.3em] uppercase mb-2" style="font-family: var(--font-wordmark)">Park</div>
      <div class="text-paper/80 text-sm" style="font-family: var(--font-display)">Select a domain to continue</div>
    </div>
  `;
  return { el, dispose: () => {} };
}

function clusterGateFromDetail(detail: unknown): ClusterGate | null {
  const source = (detail as DomainChangedDetail | null)?.source;
  if (!source || typeof source.kind !== "string") return null;
  if (source.kind === "not_in_cluster") return "not_in_cluster";
  if (source.kind === "in_cluster") return "in_cluster";
  return null;
}

function routeSig(route: Route): string {
  switch (route.view) {
    case "directory":
      return "directory";
    case "robot":
      return `robot:${route.url}`;
    case "cluster":
      return "cluster";
  }
}

function createProbe(): OverwatchParkProbe {
  return {
    snapshot: () => sdkRuntime.getCluster(),
    sensors: (peerId) => sdkRuntime.getParticipantSensors(peerId),
    debugState: () => sdkRuntime.debugState(),
    async nextSensorFrame(peerId, sensorId, maxMessages = 8) {
      const stream = await sdkRuntime.getStream(peerId, sensorId);
      try {
        for (let i = 0; i < maxMessages; i += 1) {
          const message = await stream.nextMessage();
          if (isEntryMessage(message)) {
            return {
              ...message.entry,
              payload: Array.from(toBytes(message.entry.payload)),
            };
          }
        }
        return null;
      } finally {
        await stream.close?.();
      }
    },
  };
}

function isEntryMessage(message: unknown): message is {
  entry: { payload: number[] | Uint8Array; seq?: number; timestamp_ns?: number };
} {
  return Boolean(message && typeof message === "object" && "entry" in message);
}

function toBytes(payload: number[] | Uint8Array): Uint8Array {
  if (payload instanceof Uint8Array) return payload;
  return Uint8Array.from(payload);
}
