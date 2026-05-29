import {
  type AukiBrowserBootstrapRecord,
  type AukiBrowserLocalDomain,
  type AukiBrowserPeer,
  type AukiBrowserPeerTraceEvent,
  type AukiPreviewSubscription,
  type OfferSummary,
  type PublishedByteFrame,
  type PublicationHandle,
  type PeerSummary,
  type PreviewFrame,
  LatestPublishedByteSource,
  createAukiBrowserPeer,
  getPreviewSnapshot,
  openPreviewSubscription,
  publishGeneratedPreview,
} from "@aukilabs/auki-p2p-browser";
import {
  bootstrapRecordText,
  canRequestSnapshot,
  mergeBootstrapRecords,
  parseBootstrapText,
  shortId,
} from "./app";
import "./styles.css";

type OfferRuntimeState = {
  status: string;
  snapshots: number;
  frames: number;
  totalBytes: number;
  streamFrameBase: number;
  previewUrl?: string;
  lastPayloadBytes?: number;
  lastSequence?: string;
  lastFrameAt?: Date;
  streamStartedAt?: Date;
  lastError?: string;
  getting: boolean;
  subscribing: boolean;
  stopping: boolean;
  subscription?: AukiPreviewSubscription;
  abort?: AbortController;
  stopPromise?: Promise<void>;
  token?: number;
};

type EventLogEntry = {
  at: Date;
  level: "info" | "error";
  message: string;
  detail?: string;
};

type ToastEntry = {
  id: number;
  at: Date;
  message: string;
  detail?: string;
  timeout: ReturnType<typeof window.setTimeout>;
};

type LocalPreviewPublication = {
  domain: AukiBrowserLocalDomain;
  offerId: string;
  source: LatestPublishedByteSource;
  handle: PublicationHandle;
  timer: number;
  nextSequence: number;
  generating: boolean;
  canvas: HTMLCanvasElement;
};

type AppState = {
  peer?: AukiBrowserPeer;
  peerRefreshTimer?: number;
  bootstraps: AukiBrowserBootstrapRecord[];
  peers: PeerSummary[];
  offers: OfferSummary[];
  offerStates: Map<string, OfferRuntimeState>;
  openOfferDetailKey?: string;
  openPeerDetailPeerId?: string;
  events: EventLogEntry[];
  toasts: ToastEntry[];
  localPreview?: LocalPreviewPublication;
  status: string;
  lastError?: string;
  busy: boolean;
  switchingAddress?: {
    peerId: string;
    address: string;
  };
  nextSubscriptionToken: number;
  nextToastId: number;
};

const SUBSCRIPTION_STOP_TIMEOUT_MS = 2_500;
const PEER_REFRESH_INTERVAL_MS = 500;
const ERROR_TOAST_TIMEOUT_MS = 7_000;
const MAX_ERROR_TOASTS = 3;

const state: AppState = {
  bootstraps: [],
  peers: [],
  offers: [],
  offerStates: new Map(),
  events: [],
  toasts: [],
  status: "Idle",
  busy: false,
  switchingAddress: undefined,
  nextSubscriptionToken: 0,
  nextToastId: 0,
};

const els = {
  toastRegion: element("toast-region"),
  diagnosticsButton: element<HTMLButtonElement>("diagnostics-button"),
  diagnosticsDialog: element<HTMLDialogElement>("diagnostics-dialog"),
  diagnosticsClose: element<HTMLButtonElement>("diagnostics-close"),
  startPanel: element("start-panel"),
  workspace: element("workspace"),
  streamsPanel: element("streams-panel"),
  peersPanel: element("peers-panel"),
  connectButton: element<HTMLButtonElement>("connect-button"),
  copyBootstrapButton: element<HTMLButtonElement>("copy-bootstrap-button"),
  publishPreviewButton: element<HTMLButtonElement>("publish-preview-button"),
  stopButton: element<HTMLButtonElement>("stop-button"),
  addPeerButton: element<HTMLButtonElement>("add-peer-button"),
  streamSummary: element("stream-summary"),
  streamsGrid: element("streams-grid"),
  snapshotsReceived: element("snapshots-received"),
  framesReceived: element("frames-received"),
  streamRate: element("stream-rate"),
  totalBytes: element("total-bytes"),
  lastPayloadBytes: element("last-payload-bytes"),
  lastSequence: element("last-sequence"),
  lastFrameAt: element("last-frame-at"),
  selectedOffer: element("selected-offer"),
  localPeer: element("local-peer"),
  peerCount: element("peer-count"),
  offerCount: element("offer-count"),
  lastError: element("last-error"),
  peerList: element("peer-list"),
  eventLog: element("event-log"),
  addPeerDialog: element<HTMLDialogElement>("add-peer-dialog"),
  addPeerInput: element<HTMLTextAreaElement>("add-peer-input"),
  addPeerFeedback: element("add-peer-feedback"),
  addPeerFile: element<HTMLInputElement>("add-peer-file"),
  addPeerSubmit: element<HTMLButtonElement>("add-peer-submit"),
  addPeerCancel: element<HTMLButtonElement>("add-peer-cancel"),
  addPeerClear: element<HTMLButtonElement>("add-peer-clear"),
  peerDetailDialog: element<HTMLDialogElement>("peer-detail-dialog"),
  peerDetailContent: element("peer-detail-content"),
  peerDetailClose: element<HTMLButtonElement>("peer-detail-close"),
  offerDetailDialog: element<HTMLDialogElement>("offer-detail-dialog"),
  offerDetailContent: element("offer-detail-content"),
  offerDetailClose: element<HTMLButtonElement>("offer-detail-close"),
};

els.diagnosticsButton.addEventListener("click", () => {
  els.diagnosticsDialog.showModal();
  placeToastRegion();
});
els.diagnosticsClose.addEventListener("click", () => {
  els.diagnosticsDialog.close();
});
els.connectButton.addEventListener("click", () => {
  void start();
});
els.copyBootstrapButton.addEventListener("click", () => {
  void copyLocalBootstrap();
});
els.publishPreviewButton.addEventListener("click", () => {
  void toggleGeneratedPreview();
});
els.stopButton.addEventListener("click", () => {
  void stop();
});
els.addPeerButton.addEventListener("click", () => {
  openAddPeerDialog();
});
els.addPeerSubmit.addEventListener("click", () => {
  void addPeersFromInput();
});
els.addPeerCancel.addEventListener("click", () => {
  els.addPeerDialog.close();
});
els.addPeerClear.addEventListener("click", () => {
  clearAddPeerInput();
});
els.addPeerFile.addEventListener("change", () => {
  void loadAddPeerFile();
});
els.peerDetailClose.addEventListener("click", () => {
  els.peerDetailDialog.close();
});
els.peerDetailDialog.addEventListener("close", () => {
  state.openPeerDetailPeerId = undefined;
  placeToastRegion();
});
els.diagnosticsDialog.addEventListener("close", () => {
  placeToastRegion();
});
els.addPeerDialog.addEventListener("close", () => {
  placeToastRegion();
});
els.peerDetailContent.addEventListener("pointerdown", (event) => {
  if (event.button !== 0) {
    return;
  }
  const handled = handlePeerDetailAction(event.target);
  if (handled) {
    event.preventDefault();
  }
});
els.peerDetailContent.addEventListener("keydown", (event) => {
  if (event.key !== "Enter" && event.key !== " ") {
    return;
  }
  const handled = handlePeerDetailAction(event.target);
  if (handled) {
    event.preventDefault();
  }
});
els.offerDetailClose.addEventListener("click", () => {
  els.offerDetailDialog.close();
});
els.offerDetailDialog.addEventListener("close", () => {
  state.openOfferDetailKey = undefined;
  placeToastRegion();
});
els.peerList.addEventListener("pointerdown", (event) => {
  if (event.button !== 0) {
    return;
  }
  const handled = handlePeerAction(event.target);
  if (handled) {
    event.preventDefault();
  }
});
els.peerList.addEventListener("keydown", (event) => {
  if (event.key !== "Enter" && event.key !== " ") {
    return;
  }
  const handled = handlePeerAction(event.target);
  if (handled) {
    event.preventDefault();
  }
});
els.streamsGrid.addEventListener("pointerdown", (event) => {
  if (event.button !== 0) {
    return;
  }
  const handled = handleStreamAction(event.target);
  if (handled) {
    event.preventDefault();
  }
});
els.streamsGrid.addEventListener("keydown", (event) => {
  if (event.key !== "Enter" && event.key !== " ") {
    return;
  }
  const handled = handleStreamAction(event.target);
  if (handled) {
    event.preventDefault();
  }
});
els.toastRegion.addEventListener("click", (event) => {
  const button =
    event.target instanceof Element
      ? event.target.closest<HTMLButtonElement>("button[data-toast-id]")
      : null;
  if (!button) {
    return;
  }
  dismissToast(Number(button.dataset.toastId));
});

function handleStreamAction(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) {
    return false;
  }
  const button = target.closest<HTMLButtonElement>("button[data-action][data-offer-key]");
  if (!button || button.disabled) {
    return false;
  }
  const key = decodeOfferKey(button.dataset.offerKey);
  if (!key) {
    return false;
  }
  const offer = state.offers.find((candidate) => offerKey(candidate) === key);
  if (!offer) {
    return false;
  }
  if (button.dataset.action === "get") {
    void getOfferSnapshot(offer);
  } else if (button.dataset.action === "subscribe") {
    void subscribeToOffer(offer);
  } else if (button.dataset.action === "stop-subscribe") {
    void stopOfferSubscription(key, "Stopped by user");
  } else if (button.dataset.action === "offer-detail") {
    openOfferDetail(offer);
  }
  return true;
}

function handlePeerAction(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) {
    return false;
  }
  const button = target.closest<HTMLButtonElement>("button[data-peer-id]");
  if (!button) {
    return false;
  }
  const peerId = button.dataset.peerId;
  if (!peerId) {
    return false;
  }
  openPeerDetail(peerId);
  return true;
}

function handlePeerDetailAction(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) {
    return false;
  }
  const button = target.closest<HTMLButtonElement>(
    'button[data-action="switch-address"][data-peer-id][data-address]',
  );
  if (!button || button.disabled) {
    return false;
  }
  const peerId = button.dataset.peerId;
  const address = decodeAddress(button.dataset.address);
  if (!peerId || !address) {
    return false;
  }
  void switchPeerAddress(peerId, address);
  return true;
}

render();

async function start(): Promise<void> {
  await runShortAction("Starting peer", async () => {
    await ensurePeerStarted();
    state.status = "Peer started";
  });
}

async function addPeersFromInput(): Promise<void> {
  setAddPeerFeedback("info", "Connecting...");
  await runShortAction("Adding peer", async () => {
    const records = parseBootstrapText(els.addPeerInput.value.trim());
    if (records.length === 0) {
      throw new Error("Bootstrap JSON must include at least one peer");
    }
    const peer = state.peer;
    if (!peer) {
      throw new Error("Start peer before adding remote peers");
    }
    if (records.some((record) => record.peerId === peer.peerId)) {
      throw new Error("Cannot add this browser's own bootstrap record");
    }
    recordEvent("info", "Lifecycle handshake starting", bootstrapRecordsEventDetail(records));
    await peer.connectBootstrap(records);
    state.bootstraps = mergeBootstrapRecords(state.bootstraps, records);
    state.peers = peer.listPeers();
    recordEvent("info", "Lifecycle authorized", connectedPeerEventDetail(records));
    await refreshPeerData();
    state.status = "Peer added";
    els.addPeerDialog.close();
    clearAddPeerInput();
  });
  if (state.status === "Error" && state.lastError) {
    setAddPeerFeedback("error", state.lastError);
  }
}

async function switchPeerAddress(peerId: string, address: string): Promise<void> {
  if (state.switchingAddress || state.busy) {
    return;
  }

  state.busy = true;
  state.switchingAddress = { peerId, address };
  state.status = "Switching address";
  state.lastError = undefined;
  recordEvent(
    "info",
    "Transport switch starting",
    `${peerEventDetail(peerId)} selected=${addressEventDetail(address)}`,
  );
  render();
  refreshOpenPeerDetail(peerId);

  try {
    const peer = state.peer;
    if (!peer) {
      throw new Error("Start peer before switching peer address");
    }
    if (hasActivePeerSubscription(peerId)) {
      throw new Error("Stop active streams for this peer before switching address");
    }
    await peer.switchPeerAddress(peerId, address);
    state.peers = peer.listPeers();
    state.status = "Address switched";
    recordEvent(
      "info",
      "Transport switch complete",
      `${peerEventDetail(peerId)} active=${activePathEventDetail(peerId)}`,
    );
    refreshOpenPeerDetail(peerId);
  } catch (error) {
    const message = errorMessage(error);
    state.lastError = message;
    state.status = "Error";
    recordEvent(
      "error",
      "Transport switch failed",
      `${peerEventDetail(peerId)} selected=${addressEventDetail(address)} error=${message}`,
    );
  } finally {
    state.busy = false;
    state.switchingAddress = undefined;
    render();
    refreshOpenPeerDetail(peerId);
  }
}

async function copyLocalBootstrap(): Promise<void> {
  await runShortAction("Copying bootstrap", async () => {
    const peer = state.peer;
    if (!peer) {
      throw new Error("Start peer before copying local bootstrap");
    }
    const record = await peer.localBootstrapRecord();
    await copyText(bootstrapRecordText(record));
    state.status = "Bootstrap copied";
    recordEvent("info", "Local bootstrap copied", transportSummary(record.bootstrapAddresses));
  });
}

async function toggleGeneratedPreview(): Promise<void> {
  if (state.localPreview) {
    await runShortAction("Stopping preview", async () => {
      await stopGeneratedPreview();
      await refreshPeerData();
      state.status = "Preview stopped";
      recordEvent("info", "Generated preview stopped");
    });
    return;
  }

  await runShortAction("Publishing preview", async () => {
    const peer = state.peer;
    if (!peer) {
      throw new Error("Start peer before publishing preview");
    }
    const domain = await peer.createLocalDomain({
      label: "browser-preview-domain",
      metadata: { source: "browser-generated-preview" },
    });
    const source = new LatestPublishedByteSource();
    const canvas = document.createElement("canvas");
    canvas.width = 320;
    canvas.height = 180;
    const offerId = "browser-generated-preview";
    const firstFrame = await generatedPreviewFrame(canvas, peer.peerId, 0);
    const handle = await publishGeneratedPreview(peer, source, {
      domainId: domain.domainId,
      offerId,
      displayName: "Browser Generated Preview",
      metadata: { source: "browser-generated" },
    });
    const localPreview: LocalPreviewPublication = {
      domain,
      offerId,
      source,
      handle,
      canvas,
      nextSequence: 1,
      generating: false,
      timer: 0,
    };
    source.publish(firstFrame);
    renderLocalGeneratedFrame(localPreview, firstFrame);
    localPreview.timer = window.setInterval(() => {
      if (localPreview.generating) {
        return;
      }
      localPreview.generating = true;
      void publishGeneratedFrame(localPreview, peer.peerId)
        .catch((error: unknown) => {
          const message = errorMessage(error);
          state.lastError = message;
          recordEvent("error", "Generated preview failed", message);
          window.clearInterval(localPreview.timer);
          render();
        })
        .finally(() => {
          localPreview.generating = false;
        });
    }, 100);
    state.localPreview = localPreview;
    await refreshPeerData();
    state.status = "Preview published";
    recordEvent("info", "Generated preview published", `${shortId(domain.domainId, 10)}/${offerId}`);
  });
}

function clearAddPeerInput(): void {
  els.addPeerInput.value = "";
  els.addPeerFile.value = "";
  setAddPeerFeedback("info", "");
}

function setAddPeerFeedback(level: "info" | "error", message: string): void {
  els.addPeerFeedback.textContent = message;
  els.addPeerFeedback.className = `form-feedback ${message ? level : ""}`.trim();
}

async function getOfferSnapshot(offer: OfferSummary): Promise<void> {
  const peer = state.peer;
  if (!peer || !offer.accessModes.includes("get")) {
    return;
  }

  const key = offerKey(offer);
  const runtime = offerRuntime(key);
  if (!canRequestSnapshot(Boolean(peer), runtime)) {
    return;
  }

  const startedAt = performance.now();
  runtime.getting = true;
  runtime.status = "getting";
  runtime.lastError = undefined;
  state.status = "Getting snapshot";
  state.lastError = undefined;
  recordEvent(
    "info",
    "Get requested",
    `${offerEventDetail(offer)} active=${activePathEventDetail(offer.peerId)}`,
  );
  render();
  updateOpenOfferDetail(key);

  try {
    const frame = await getPreviewSnapshot(peer, offer);
    const bytes = renderOfferFrame(key, runtime, frame);
    runtime.snapshots += 1;
    runtime.totalBytes += bytes;
    runtime.status = "snapshot ok";
    state.status = "Snapshot received";
    updateOpenOfferDetail(key);
    recordEvent(
      "info",
      "Get snapshot received",
      `${offerEventDetail(offer)} bytes=${bytes} sequence=${frame.sequence ?? "unknown"} duration=${Math.round(performance.now() - startedAt)}ms`,
    );
  } catch (error) {
    const message = errorMessage(error);
    runtime.status = "error";
    runtime.lastError = message;
    state.lastError = message;
    state.status = "Error";
    updateOpenOfferDetail(key);
    recordEvent("error", "Get failed", `${offerEventDetail(offer)} error=${message}`);
  } finally {
    runtime.getting = false;
    render();
    updateOpenOfferDetail(key);
  }
}

async function subscribeToOffer(offer: OfferSummary): Promise<void> {
  const peer = state.peer;
  if (!peer || !offer.accessModes.includes("subscribe")) {
    return;
  }

  const key = offerKey(offer);
  const runtime = offerRuntime(key);
  if (runtime.subscription || runtime.subscribing || runtime.stopping) {
    return;
  }

  const token = ++state.nextSubscriptionToken;
  const abortController = new AbortController();
  runtime.token = token;
  runtime.abort = abortController;
  runtime.subscribing = true;
  runtime.status = "subscribing";
  runtime.lastError = undefined;
  state.status = "Subscribing";
  state.lastError = undefined;
  recordEvent(
    "info",
    "Subscribe opening",
    `${offerEventDetail(offer)} active=${activePathEventDetail(offer.peerId)}`,
  );
  render();
  updateOpenOfferDetail(key);

  try {
    const subscription = await openPreviewSubscription(peer, offer, {
      signal: abortController.signal,
    });
    if (runtime.token !== token) {
      await subscription.stop().catch(() => undefined);
      return;
    }

    runtime.subscription = subscription;
    runtime.subscribing = false;
    runtime.status = "streaming";
    runtime.streamStartedAt = new Date();
    runtime.streamFrameBase = runtime.frames;
    state.status = "Receiving";
    recordEvent("info", "Subscribe accepted", offerEventDetail(offer));
    render();
    updateOpenOfferDetail(key);

    let firstFrame = true;
    for await (const frame of subscription.frames) {
      if (runtime.token !== token) {
        break;
      }
      const bytes = renderOfferFrame(key, runtime, frame);
      runtime.frames += 1;
      runtime.totalBytes += bytes;
      runtime.status = "streaming";
      state.status = "Receiving";
      updateStreamCardRuntime(key, runtime);
      updateOpenOfferDetail(key);
      renderLiveStats();
      if (firstFrame) {
        firstFrame = false;
        recordEvent(
          "info",
          "Subscribe first frame",
          `${offerEventDetail(offer)} bytes=${bytes} sequence=${frame.sequence ?? "unknown"}`,
        );
      }
    }

    if (runtime.token === token) {
      clearRuntimeSubscription(runtime);
      runtime.status = "complete";
      state.status = "Subscription complete";
      recordEvent(
        "info",
        "Subscribe stream closed",
        `${offerEventDetail(offer)} reason=complete frames=${runtime.frames}`,
      );
      render();
      updateOpenOfferDetail(key);
    }
  } catch (error) {
    if (runtime.token !== token) {
      return;
    }
    const message = errorMessage(error);
    clearRuntimeSubscription(runtime);
    runtime.status = "error";
    runtime.lastError = message;
    state.lastError = message;
    state.status = "Error";
    recordEvent("error", "Subscribe failed", `${offerEventDetail(offer)} error=${message}`);
    render();
    updateOpenOfferDetail(key);
  }
}

async function stop(): Promise<void> {
  await runShortAction("Stopping", async () => {
    await stopPeer();
    state.status = "Stopped";
    recordEvent("info", "Browser peer stopped");
  });
}

async function stopOfferSubscription(key: string, reason: string): Promise<void> {
  const runtime = state.offerStates.get(key);
  if (!runtime) {
    return;
  }
  if (runtime.stopPromise) {
    await runtime.stopPromise;
    return;
  }
  if (!runtime.subscription && !runtime.abort && !runtime.subscribing) {
    return;
  }

  const stop = stopOfferSubscriptionOnce(key, runtime, reason);
  runtime.stopPromise = stop;
  try {
    await stop;
  } finally {
    if (runtime.stopPromise === stop) {
      runtime.stopPromise = undefined;
    }
  }
}

async function stopOfferSubscriptionOnce(
  key: string,
  runtime: OfferRuntimeState,
  reason: string,
): Promise<void> {
  const subscription = runtime.subscription;
  const abortController = runtime.abort;
  runtime.token = ++state.nextSubscriptionToken;
  runtime.stopping = true;
  runtime.subscribing = false;
  runtime.status = "stopping";
  runtime.lastError = undefined;
  state.status = "Stopping subscription";
  const offer = offerByKey(key);
  recordEvent(
    "info",
    "Subscribe stop requested",
    `${offer ? offerEventDetail(offer) : `offer=${key}`} reason=${reason}`,
  );
  render();
  updateOpenOfferDetail(key);

  try {
    await stopSubscriptionTransport(subscription, abortController, reason);
  } catch (error) {
    const message = errorMessage(error);
    runtime.lastError = message;
    state.lastError = message;
    recordEvent("error", "Subscribe stop failed", message);
  } finally {
    abortController?.abort(new Error(reason));
    clearRuntimeSubscription(runtime);
    runtime.stopping = false;
    runtime.status = "stopped";
    state.status = "Subscription stopped";
    recordEvent(
      "info",
      "Subscribe stream closed",
      `${offer ? offerEventDetail(offer) : `offer=${key}`} reason=cancelled`,
    );
    render();
    updateOpenOfferDetail(key);
  }
}

async function stopPeer(): Promise<void> {
  stopPeerObserver();
  await stopGeneratedPreview();
  const stops = Array.from(state.offerStates.entries())
    .filter(([, runtime]) => runtime.subscription || runtime.abort || runtime.subscribing)
    .map(([key, runtime]) =>
      stopOfferSubscriptionOnce(key, runtime, "Peer stopped").catch(() => undefined),
    );
  await Promise.all(stops);
  if (state.peer) {
    await state.peer.stop();
  }
  clearAllPreviewFrames();
  state.peer = undefined;
  state.bootstraps = [];
  state.peers = [];
  state.offers = [];
  state.offerStates.clear();
  state.localPreview = undefined;
  state.lastError = undefined;
  state.nextSubscriptionToken += 1;
}

async function ensurePeerStarted(bootstrap?: unknown): Promise<AukiBrowserPeer> {
  if (state.peer) {
    return state.peer;
  }
  const peer = await createAukiBrowserPeer({
    bootstrap,
    label: "p2p-preview-browser",
    trace: handlePeerTrace,
  });
  state.peer = peer;
  startPeerObserver();
  state.status = "Peer started";
  recordEvent("info", "Browser peer started", shortId(peer.peerId, 12));
  return peer;
}

async function refreshPeerData(): Promise<void> {
  const peer = state.peer;
  if (!peer) {
    state.peers = [];
    state.offers = [];
    return;
  }
  const peers = peer.listPeers();
  state.peers = peers;
  if (peers.length > 0) {
    recordEvent("info", "Offer catalog requesting", peerListEventDetail(peers));
  }
  state.offers = await peer.listOffers();
  recordEvent("info", "Offer catalog loaded", offerCatalogEventDetail(state.offers, peers));
}

function startPeerObserver(): void {
  if (state.peerRefreshTimer !== undefined) {
    return;
  }
  state.peerRefreshTimer = window.setInterval(refreshPeerStateFromSdk, PEER_REFRESH_INTERVAL_MS);
}

function stopPeerObserver(): void {
  if (state.peerRefreshTimer === undefined) {
    return;
  }
  window.clearInterval(state.peerRefreshTimer);
  state.peerRefreshTimer = undefined;
}

function refreshPeerStateFromSdk(): void {
  const peer = state.peer;
  if (!peer) {
    return;
  }

  const previousPeers = new Map(state.peers.map((summary) => [summary.peerId, summary]));
  const peers = peer.listPeers();
  const observedPeers = peers.filter((summary) => !previousPeers.has(summary.peerId));
  let peerDisplayChanged = observedPeers.length > 0;
  state.peers = peers;

  for (const summary of peers) {
    const previous = previousPeers.get(summary.peerId);
    if (!previous) {
      recordEvent(
        "info",
        "Peer observed",
        `${peerEventDetail(summary.peerId)} active=${connectionPathSummary(summary.connectionPaths)}`,
      );
      continue;
    }

    if (previous.connected !== summary.connected) {
      peerDisplayChanged = true;
      const detail = `${peerEventDetail(summary.peerId)} active=${connectionPathSummary(summary.connectionPaths)}`;
      if (summary.connected) {
        if (state.lastError?.startsWith("Peer disconnected:")) {
          state.lastError = undefined;
        }
      } else {
        state.lastError = `Peer disconnected: ${shortId(summary.peerId, 12)}`;
      }
      recordEvent(
        summary.connected ? "info" : "error",
        summary.connected ? "Peer connected" : "Peer disconnected",
        detail,
      );
      continue;
    }

    if (
      connectionPathSummary(previous.connectionPaths) !==
      connectionPathSummary(summary.connectionPaths)
    ) {
      peerDisplayChanged = true;
      recordEvent(
        "info",
        "Peer path changed",
        `${peerEventDetail(summary.peerId)} active=${connectionPathSummary(summary.connectionPaths)}`,
      );
    }
  }

  renderLiveStats();
  renderPeerSidebar(state.peers);
  if (peerDisplayChanged) {
    renderStreams(state.offers);
  }
  if (state.openPeerDetailPeerId) {
    refreshOpenPeerDetail(state.openPeerDetailPeerId);
  }
  renderEvents();
  if (observedPeers.length > 0) {
    void refreshOffersAfterPeerObservation(observedPeers);
  }
}

async function refreshOffersAfterPeerObservation(observedPeers: PeerSummary[]): Promise<void> {
  try {
    await refreshPeerData();
    render();
  } catch (error) {
    const message = errorMessage(error);
    state.lastError = message;
    recordEvent(
      "error",
      "Offer refresh failed",
      `${peerListEventDetail(observedPeers)} error=${message}`,
    );
    renderLiveStats();
    renderEvents();
  }
}

function openAddPeerDialog(): void {
  if (!state.peer) {
    return;
  }
  els.addPeerDialog.showModal();
  placeToastRegion();
  els.addPeerInput.focus();
}

async function loadAddPeerFile(): Promise<void> {
  const file = els.addPeerFile.files?.[0];
  if (!file) {
    return;
  }
  els.addPeerInput.value = await file.text();
  try {
    parseBootstrapText(els.addPeerInput.value.trim());
    state.lastError = undefined;
    recordEvent("info", "Bootstrap JSON loaded", file.name);
  } catch (error) {
    state.lastError = errorMessage(error);
    recordEvent("error", "Bootstrap JSON invalid", state.lastError);
  }
  render();
}

async function runShortAction(label: string, action: () => Promise<void>): Promise<void> {
  state.busy = true;
  state.status = label;
  state.lastError = undefined;
  render();
  try {
    await action();
  } catch (error) {
    const message = errorMessage(error);
    state.lastError = message;
    state.status = "Error";
    recordEvent("error", label, message);
  } finally {
    state.busy = false;
    render();
  }
}

function renderOfferFrame(
  key: string,
  runtime: OfferRuntimeState,
  frame: PreviewFrame,
): number {
  const bytes = frame.bytes;
  const buffer = bytes.buffer.slice(
    bytes.byteOffset,
    bytes.byteOffset + bytes.byteLength,
  ) as ArrayBuffer;
  const previousUrl = runtime.previewUrl;
  runtime.previewUrl = URL.createObjectURL(new Blob([buffer], { type: "image/jpeg" }));
  runtime.lastPayloadBytes = bytes.byteLength;
  runtime.lastSequence = frame.sequence;
  runtime.lastFrameAt = new Date();
  updateStreamCardImage(key, runtime);
  if (previousUrl) {
    window.setTimeout(() => URL.revokeObjectURL(previousUrl), 1_000);
  }
  return bytes.byteLength;
}

function clearRuntimeSubscription(runtime: OfferRuntimeState): void {
  runtime.subscription = undefined;
  runtime.abort = undefined;
  runtime.subscribing = false;
  runtime.stopping = false;
  runtime.stopPromise = undefined;
  runtime.token = undefined;
}

function clearAllPreviewFrames(): void {
  for (const runtime of state.offerStates.values()) {
    if (runtime.previewUrl) {
      URL.revokeObjectURL(runtime.previewUrl);
      runtime.previewUrl = undefined;
    }
  }
}

function render(): void {
  renderLiveStats();
  renderPeerSidebar(state.peers);
  renderStreams(state.offers);
  renderEvents();
  renderToasts();
}

function renderLiveStats(): void {
  const totals = aggregateRuntimeStats();
  const hasPeer = Boolean(state.peer);
  const hasRemoteContext = state.peers.length > 0 || state.offers.length > 0;
  const canCopyBootstrap = hasPeer && state.peers.length > 0;
  els.startPanel.hidden = hasPeer;
  els.workspace.hidden = !hasPeer;
  els.workspace.classList.toggle("peer-only", !hasRemoteContext);
  els.peersPanel.hidden = !hasPeer;
  els.streamsPanel.hidden = !hasRemoteContext;
  els.diagnosticsButton.hidden = !hasPeer;
  els.copyBootstrapButton.hidden = !canCopyBootstrap;
  els.publishPreviewButton.hidden = !hasPeer;
  els.stopButton.hidden = !hasPeer;
  els.connectButton.hidden = hasPeer;
  els.snapshotsReceived.textContent = totals.snapshots.toString();
  els.framesReceived.textContent = totals.frames.toString();
  els.streamRate.textContent = `${totals.rate.toFixed(1)} fps`;
  els.totalBytes.textContent = formatBytes(totals.totalBytes);
  els.lastPayloadBytes.textContent =
    totals.lastPayloadBytes === undefined ? "None" : formatBytes(totals.lastPayloadBytes);
  els.lastSequence.textContent = totals.lastSequence ?? "None";
  els.lastFrameAt.textContent = totals.lastFrameAt
    ? totals.lastFrameAt.toLocaleTimeString()
    : "Never";
  els.selectedOffer.textContent = totals.activeStreams.toString();
  els.localPeer.textContent = state.peer ? shortId(state.peer.peerId, 10) : "Not started";
  els.peerCount.textContent = (state.peers.length + (state.peer ? 1 : 0)).toString();
  els.offerCount.textContent = state.offers.length.toString();
  els.lastError.textContent = state.lastError ?? "None";
  els.streamSummary.textContent =
    state.offers.length === 0
      ? "No offers"
      : `${totals.activeStreams} active / ${state.offers.length} offer(s)`;
  els.connectButton.disabled = state.busy || hasPeer;
  els.connectButton.textContent = "Start Peer";
  els.copyBootstrapButton.disabled = state.busy || !canCopyBootstrap;
  els.copyBootstrapButton.textContent = "Copy Bootstrap";
  els.publishPreviewButton.disabled = state.busy || !state.peer;
  els.publishPreviewButton.textContent = state.localPreview ? "Stop Preview" : "Publish Preview";
  els.stopButton.disabled = state.busy || !state.peer;
  els.addPeerButton.disabled = state.busy || !state.peer;
  els.addPeerButton.textContent = "Add Peer";
  els.addPeerSubmit.disabled = state.busy || !state.peer;
}

function renderPeerSidebar(peers: PeerSummary[]): void {
  els.peerList.replaceChildren();
  if (state.peer) {
    els.peerList.append(localPeerItem(state.peer.peerId));
  }
  for (const peer of peers) {
    els.peerList.append(remotePeerItem(peer));
  }
  if (!state.peer && peers.length === 0) {
    const empty = document.createElement("div");
    empty.className = "empty-peers";
    empty.textContent = "Start peer";
    els.peerList.append(empty);
  }
}

function localPeerItem(peerId: string): HTMLElement {
  const button = peerListItem(peerId, "Local", "browser", state.peer?.multiaddrs() ?? []);
  button.classList.add("local");
  return button;
}

function remotePeerItem(peer: PeerSummary): HTMLElement {
  const offerCount = state.offers.filter((offer) => offer.peerId === peer.peerId).length;
  const status = peer.connected ? "Connected" : "Disconnected";
  const addresses = uniqueStrings([...peer.dialAddresses, ...peer.observedAddresses]);
  return peerListItem(
    peer.peerId,
    status,
    `${offerCount} offer(s) | active ${connectionPathSummary(peer.connectionPaths)}`,
    addresses,
    peer.connected,
  );
}

function peerListItem(
  peerId: string,
  status: string,
  detail: string,
  addresses: string[],
  connected = true,
): HTMLButtonElement {
  const button = document.createElement("button");
  button.type = "button";
  button.className = "peer-list-item";
  button.dataset.peerId = peerId;

  const heading = document.createElement("span");
  heading.className = "peer-list-heading";
  const title = document.createElement("strong");
  title.textContent = shortId(peerId, 10);
  const statusPill = document.createElement("span");
  statusPill.className = connected ? "status-pill live" : "status-pill error";
  statusPill.textContent = status;
  heading.append(title, statusPill);

  const meta = document.createElement("span");
  meta.className = "peer-list-meta";
  meta.textContent = `${detail} | ${addresses.length} addr | ${transportSummary(addresses)}`;

  button.append(heading, meta);
  return button;
}

function openPeerDetail(peerId: string): void {
  const remotePeer = state.peers.find((peer) => peer.peerId === peerId);
  const isLocal = state.peer?.peerId === peerId;
  if (!remotePeer && !isLocal) {
    return;
  }

  state.openPeerDetailPeerId = peerId;
  els.peerDetailContent.replaceChildren(peerDetail(peerId, remotePeer, isLocal));
  els.peerDetailDialog.showModal();
  placeToastRegion();
}

function refreshOpenPeerDetail(peerId: string): void {
  if (!els.peerDetailDialog.open) {
    return;
  }
  const remotePeer = state.peers.find((peer) => peer.peerId === peerId);
  const isLocal = state.peer?.peerId === peerId;
  if (!remotePeer && !isLocal) {
    els.peerDetailDialog.close();
    state.openPeerDetailPeerId = undefined;
    return;
  }
  els.peerDetailContent.replaceChildren(peerDetail(peerId, remotePeer, isLocal));
}

function hasActivePeerSubscription(peerId: string): boolean {
  return state.offers.some((offer) => {
    if (offer.peerId !== peerId) {
      return false;
    }
    const runtime = state.offerStates.get(offerKey(offer));
    return Boolean(
      runtime?.subscription ||
        runtime?.subscribing ||
        runtime?.stopping,
    );
  });
}

function peerDetail(
  peerId: string,
  peer: PeerSummary | undefined,
  isLocal: boolean,
): HTMLElement {
  const wrapper = document.createElement("div");
  wrapper.className = "peer-detail";
  const offers = state.offers.filter((offer) => offer.peerId === peerId);
  const bootstrap = state.bootstraps.find((record) => record.peerId === peerId);
  const addresses = isLocal ? state.peer?.multiaddrs() ?? [] : peer?.dialAddresses ?? [];
  const observedAddresses = isLocal ? [] : peer?.observedAddresses ?? [];
  const bootstrapAddresses = bootstrapAddressList(bootstrap);
  const connectionPaths = isLocal ? [] : peer?.connectionPaths ?? [];
  const visibleAddresses = uniqueStrings([...addresses, ...observedAddresses]);

  const summary = document.createElement("div");
  summary.className = "peer-detail-grid";
  summary.append(
    detailMetric("Peer ID", peerId),
    detailMetric("Role", isLocal ? "Browser" : "Remote"),
    detailMetric("Connected", isLocal || peer?.connected ? "Yes" : "No"),
    detailMetric("Active Transport", connectionPathSummary(connectionPaths)),
    detailMetric("Connection Paths", connectionPaths.length.toString()),
    detailMetric("Dial Addresses", addresses.length.toString()),
    detailMetric("Observed Addresses", observedAddresses.length.toString()),
    detailMetric("Known Transports", transportSummary(visibleAddresses)),
    detailMetric("Offers", offers.length.toString()),
  );

  wrapper.append(summary);
  wrapper.append(
    addressInventorySection(
      peerId,
      isLocal,
      addresses,
      observedAddresses,
      bootstrapAddresses,
      connectionPaths,
    ),
  );
  wrapper.append(offerSection(offers));
  return wrapper;
}

function detailMetric(label: string, value: string, key?: string): HTMLElement {
  const item = document.createElement("div");
  if (key) {
    item.dataset.metric = key;
  }
  const name = document.createElement("span");
  name.textContent = label;
  const content = document.createElement("strong");
  content.textContent = value;
  item.append(name, content);
  return item;
}

function addressInventorySection(
  peerId: string,
  isLocal: boolean,
  dialAddresses: string[],
  observedAddresses: string[],
  bootstrapAddresses: string[],
  paths: PeerSummary["connectionPaths"],
): HTMLElement {
  const section = document.createElement("section");
  section.className = "detail-section";
  const heading = document.createElement("h3");
  heading.textContent = "Addresses";
  const list = document.createElement("div");
  list.className = "address-inventory";
  const entries = addressInventory(dialAddresses, observedAddresses, bootstrapAddresses, paths);
  if (entries.length === 0) {
    list.textContent = "None";
  } else {
    const activePathCount = paths.length;
    for (const entry of entries) {
      list.append(
        addressInventoryRow(peerId, entry, !isLocal, activePathCount),
      );
    }
  }
  section.append(heading, list);
  return section;
}

type AddressInventoryEntry = {
  address: string;
  roles: Set<"active" | "dial" | "observed" | "bootstrap">;
  paths: PeerSummary["connectionPaths"];
};

function addressInventory(
  dialAddresses: string[],
  observedAddresses: string[],
  bootstrapAddresses: string[],
  paths: PeerSummary["connectionPaths"],
): AddressInventoryEntry[] {
  const byAddress = new Map<string, AddressInventoryEntry>();
  const upsert = (address: string): AddressInventoryEntry => {
    let entry = byAddress.get(address);
    if (!entry) {
      entry = { address, roles: new Set(), paths: [] };
      byAddress.set(address, entry);
    }
    return entry;
  };

  for (const path of paths) {
    const entry = upsert(path.remoteAddress);
    entry.roles.add("active");
    entry.paths.push(path);
  }
  for (const address of dialAddresses) {
    upsert(address).roles.add("dial");
  }
  for (const address of observedAddresses) {
    upsert(address).roles.add("observed");
  }
  for (const address of bootstrapAddresses) {
    upsert(address).roles.add("bootstrap");
  }

  return Array.from(byAddress.values());
}

function addressInventoryRow(
  peerId: string,
  entry: AddressInventoryEntry,
  canSwitch: boolean,
  activePathCount: number,
): HTMLElement {
  const row = document.createElement("div");
  row.className = "address-row";

  const flags = document.createElement("div");
  flags.className = "address-flags";
  for (const role of entry.roles) {
    const flag = document.createElement("span");
    flag.className = `address-flag ${role}`;
    flag.textContent = role;
    flags.append(flag);
  }

  const content = document.createElement("div");
  content.className = "address-content";
  const meta = document.createElement("span");
  meta.textContent = entry.paths.length > 0
    ? `${entry.paths.length} active connection path${entry.paths.length === 1 ? "" : "s"}`
    : `dialable | ${transportSummary([entry.address])}`;
  content.append(meta, addressValue("Address", entry.address, true));
  for (const path of entry.paths) {
    content.append(connectionPathDetail(path));
  }

  const action = document.createElement("div");
  action.className = "address-action";
  if (canSwitch && entry.roles.has("dial")) {
    const switching = state.switchingAddress;
    const isSwitching =
      switching?.peerId === peerId && switching.address === entry.address;
    const isActive = entry.roles.has("active");
    const isOnlyActive = isActive && activePathCount === 1;
    const button = document.createElement("button");
    button.type = "button";
    button.dataset.action = "switch-address";
    button.dataset.peerId = peerId;
    button.dataset.address = encodeAddress(entry.address);
    button.disabled =
      isSwitching ||
      (state.busy && !isSwitching) ||
      (Boolean(state.switchingAddress) && !isSwitching) ||
      isOnlyActive;
    if (isSwitching) {
      button.classList.add("loading");
      button.setAttribute("aria-busy", "true");
    }
    button.textContent = isSwitching
      ? "Switching"
      : isOnlyActive
        ? selectedAddressLabel(entry.address, "Using")
        : selectedAddressLabel(entry.address, "Use");
    action.append(button);
  }

  row.append(flags, content, action);
  return row;
}

function connectionPathDetail(path: PeerSummary["connectionPaths"][number]): HTMLElement {
  const detail = document.createElement("div");
  detail.className = "connection-path-detail";
  detail.append(
    addressValue("Transport", formatTransportName(path.transport)),
    addressValue("Direction", path.direction),
    addressValue("Path", connectionPathKind(path)),
    addressValue("Status", path.status),
    addressValue("Connection ID", path.connectionId, true),
    addressValue(
      "RTT",
      path.rttMs === undefined ? "not reported" : `${Math.round(path.rttMs)} ms`,
    ),
  );
  return detail;
}

function selectedAddressLabel(address: string, prefix: "Use" | "Using"): string {
  if (address.includes("/p2p-circuit/webrtc/p2p/")) {
    return `${prefix} WebRTC`;
  }
  if (address.includes("/p2p-circuit/p2p/")) {
    return `${prefix} Relay`;
  }
  if (address.includes("/webrtc-direct")) {
    return `${prefix} WebRTC Direct`;
  }
  if (address.includes("/ws") || address.includes("/wss")) {
    return `${prefix} WebSocket`;
  }
  return prefix;
}

function addressValue(label: string, value: string, code = false): HTMLElement {
  const item = document.createElement("div");
  item.className = "address-value";
  const name = document.createElement("span");
  name.textContent = label;
  const content = code ? document.createElement("code") : document.createElement("strong");
  content.textContent = value;
  item.append(name, content);
  return item;
}

function offerSection(offers: OfferSummary[]): HTMLElement {
  const section = document.createElement("section");
  section.className = "detail-section";
  const heading = document.createElement("h3");
  heading.textContent = "Offers";
  const list = document.createElement("div");
  list.className = "offer-detail-list";
  if (offers.length === 0) {
    list.textContent = "None";
  } else {
    for (const offer of offers) {
      const item = document.createElement("div");
      item.textContent = `${offer.domainId}/${offer.offerId} | ${offer.kind} | ${offer.accessModes.join(", ")}`;
      list.append(item);
    }
  }
  section.append(heading, list);
  return section;
}

function renderStreams(offers: OfferSummary[]): void {
  els.streamsGrid.replaceChildren();
  if (offers.length === 0) {
    const empty = document.createElement("div");
    empty.className = "empty-streams";
    empty.textContent = emptyOffersMessage();
    els.streamsGrid.append(empty);
    return;
  }

  for (const [peerId, peerOffers] of offersByPeer(offers)) {
    const group = document.createElement("section");
    group.className = "peer-stream-group";

    const header = document.createElement("div");
    header.className = "peer-stream-heading";
    const title = document.createElement("h3");
    title.textContent = shortId(peerId, 10);
    const count = document.createElement("span");
    count.textContent = `${peerOffers.length} offer(s)`;
    header.append(title, count);
    group.append(header);

    const grid = document.createElement("div");
    grid.className = "stream-cards";
    for (const offer of peerOffers) {
      grid.append(streamCard(offer));
    }
    group.append(grid);
    els.streamsGrid.append(group);
  }
}

function emptyOffersMessage(): string {
  if (!state.peer) {
    return "Start peer to begin";
  }
  if (state.peers.length === 0) {
    return "Add peer to load offers";
  }
  return "No offers loaded";
}

function streamCard(offer: OfferSummary): HTMLElement {
  const key = offerKey(offer);
  const runtime = offerRuntime(key);
  const isLocal = offer.peerId === state.peer?.peerId;
  const peerConnected = isOfferPeerConnected(offer);
  const disconnected = !isLocal && !peerConnected;
  const card = document.createElement("article");
  card.className = "stream-card";
  card.dataset.offerCard = encodeOfferKey(key);
  if (runtime.subscription || runtime.subscribing) {
    card.classList.add("streaming");
  }
  if (isLocal) {
    card.classList.add("local-offer");
  }

  const frame = document.createElement("div");
  frame.className = "stream-frame";
  if (runtime.previewUrl) {
    const image = document.createElement("img");
    image.src = runtime.previewUrl;
    image.alt = "";
    frame.append(image);
  } else {
    const empty = document.createElement("div");
    empty.className = "stream-empty";
    empty.textContent = "No frame";
    frame.append(empty);
  }

  const body = document.createElement("div");
  body.className = "stream-body";

  const header = document.createElement("div");
  header.className = "stream-card-heading";
  const title = document.createElement("h3");
  title.textContent = offer.offerId;
  const status = document.createElement("span");
  status.className = `status-pill ${offerStatusClass(offer, runtime)}`;
  status.dataset.role = "status";
  status.textContent = offerStatusText(offer, runtime);
  header.append(title, status);

  const meta = document.createElement("div");
  meta.className = "stream-meta";
  meta.append(
    metric("Domain", shortId(offer.domainId, 7)),
    metric("Payload", offer.payloadType ?? "unknown"),
  );

  const actions = document.createElement("div");
  actions.className = "row-actions";
  actions.append(actionButton("Detail", "offer-detail", key, false));
  if (offer.accessModes.includes("get")) {
    actions.append(
      actionButton(
        runtime.getting ? "Getting" : "Get",
        "get",
        key,
        isLocal || disconnected || state.busy || !canRequestSnapshot(Boolean(state.peer), runtime),
      ),
    );
  }
  if (offer.accessModes.includes("subscribe")) {
    const active = Boolean(runtime.subscription || runtime.subscribing || runtime.stopping);
    const label = runtime.stopping ? "Stopping" : active ? "Stop" : "Subscribe";
    const action = active ? "stop-subscribe" : "subscribe";
    const button = actionButton(
      label,
      action,
      key,
      isLocal || disconnected || state.busy || !state.peer || runtime.stopping || runtime.getting,
    );
    if (runtime.stopping || runtime.subscribing) {
      button.classList.add("loading");
      button.setAttribute("aria-busy", "true");
    }
    actions.append(button);
  }

  if (runtime.lastError || disconnected) {
    const error = document.createElement("div");
    error.className = "stream-error";
    error.textContent = runtime.lastError ?? "Peer disconnected";
    body.append(header, meta, actions, error);
  } else {
    body.append(header, meta, actions);
  }
  card.append(frame, body);
  return card;
}

function openOfferDetail(offer: OfferSummary): void {
  state.openOfferDetailKey = offerKey(offer);
  els.offerDetailContent.replaceChildren(offerDetail(offer));
  els.offerDetailDialog.showModal();
  placeToastRegion();
}

function offerDetail(offer: OfferSummary): HTMLElement {
  const key = offerKey(offer);
  const runtime = offerRuntime(key);
  const wrapper = document.createElement("div");
  wrapper.className = "peer-detail";
  wrapper.dataset.offerDetail = encodeOfferKey(key);

  const summary = document.createElement("div");
  summary.className = "peer-detail-grid";
  summary.append(
    detailMetric("Peer ID", offer.peerId),
    detailMetric("Domain ID", offer.domainId),
    detailMetric("Offer ID", offer.offerId),
    detailMetric("Kind", offer.kind ?? "unknown"),
    detailMetric("Payload", offer.payloadType ?? "unknown"),
    detailMetric("Access", offer.accessModes.join(", ")),
    detailMetric("Status", runtime.status, "status"),
    detailMetric("Frames", runtime.frames.toString(), "frames"),
    detailMetric("Rate", `${streamRate(runtime).toFixed(1)} fps`, "rate"),
    detailMetric("Gets", runtime.snapshots.toString(), "gets"),
    detailMetric("Total Bytes", formatBytes(runtime.totalBytes), "bytes"),
    detailMetric(
      "Last Payload",
      runtime.lastPayloadBytes === undefined ? "None" : formatBytes(runtime.lastPayloadBytes),
      "payload",
    ),
    detailMetric("Sequence", runtime.lastSequence ?? "None", "sequence"),
    detailMetric(
      "Last Frame",
      runtime.lastFrameAt ? runtime.lastFrameAt.toLocaleTimeString() : "Never",
      "last-frame",
    ),
  );
  wrapper.append(summary);

  if (runtime.lastError) {
    const section = document.createElement("section");
    section.className = "detail-section";
    const heading = document.createElement("h3");
    heading.textContent = "Last Error";
    const message = document.createElement("pre");
    message.textContent = runtime.lastError;
    section.append(heading, message);
    wrapper.append(section);
  }

  return wrapper;
}

function metric(label: string, value: string, key?: string): HTMLElement {
  const item = document.createElement("div");
  if (key) {
    item.dataset.metric = key;
  }
  const name = document.createElement("span");
  name.textContent = label;
  const content = document.createElement("strong");
  content.textContent = value;
  item.append(name, content);
  return item;
}

function updateStreamCardImage(key: string, runtime: OfferRuntimeState): void {
  if (!runtime.previewUrl) {
    return;
  }
  const card = streamCardElement(key);
  const frame = card?.querySelector<HTMLElement>(".stream-frame");
  if (!frame) {
    return;
  }
  let image = frame.querySelector<HTMLImageElement>("img");
  if (!image) {
    image = document.createElement("img");
    image.alt = "";
    frame.replaceChildren(image);
  }
  if (image.src !== runtime.previewUrl) {
    image.src = runtime.previewUrl;
  }
}

function updateStreamCardRuntime(key: string, runtime: OfferRuntimeState): void {
  const card = streamCardElement(key);
  const offer = offerByKey(key);
  if (!card) {
    return;
  }
  card.classList.toggle("streaming", Boolean(runtime.subscription || runtime.subscribing));
  const status = card.querySelector<HTMLElement>('[data-role="status"]');
  if (status) {
    status.className = `status-pill ${
      offer ? offerStatusClass(offer, runtime) : statusClass(runtime)
    }`;
    status.textContent = offer ? offerStatusText(offer, runtime) : runtime.status;
  }
  setMetric(card, "snapshots", runtime.snapshots.toString());
  setMetric(card, "frames", runtime.frames.toString());
  setMetric(card, "rate", `${streamRate(runtime).toFixed(1)} fps`);
  setMetric(card, "bytes", formatBytes(runtime.totalBytes));
  setMetric(
    card,
    "payload",
    runtime.lastPayloadBytes === undefined ? "None" : formatBytes(runtime.lastPayloadBytes),
  );
  setMetric(card, "sequence", runtime.lastSequence ?? "None");
  setMetric(
    card,
    "last-frame",
    runtime.lastFrameAt ? runtime.lastFrameAt.toLocaleTimeString() : "Never",
  );
}

function updateOpenOfferDetail(key: string): void {
  if (state.openOfferDetailKey !== key || !els.offerDetailDialog.open) {
    return;
  }
  const runtime = state.offerStates.get(key);
  if (!runtime) {
    return;
  }
  const container = els.offerDetailContent.querySelector<HTMLElement>(
    `[data-offer-detail="${encodeOfferKey(key)}"]`,
  );
  if (!container) {
    return;
  }
  setMetric(container, "status", runtime.status);
  setMetric(container, "frames", runtime.frames.toString());
  setMetric(container, "rate", `${streamRate(runtime).toFixed(1)} fps`);
  setMetric(container, "gets", runtime.snapshots.toString());
  setMetric(container, "bytes", formatBytes(runtime.totalBytes));
  setMetric(
    container,
    "payload",
    runtime.lastPayloadBytes === undefined ? "None" : formatBytes(runtime.lastPayloadBytes),
  );
  setMetric(container, "sequence", runtime.lastSequence ?? "None");
  setMetric(
    container,
    "last-frame",
    runtime.lastFrameAt ? runtime.lastFrameAt.toLocaleTimeString() : "Never",
  );
}

function setMetric(card: HTMLElement, metricKey: string, value: string): void {
  const target = card.querySelector<HTMLElement>(`[data-metric="${metricKey}"] strong`);
  if (target) {
    target.textContent = value;
  }
}

function streamCardElement(key: string): HTMLElement | undefined {
  const encoded = encodeOfferKey(key);
  return Array.from(
    els.streamsGrid.querySelectorAll<HTMLElement>("[data-offer-card]"),
  ).find((element) => element.dataset.offerCard === encoded);
}

function actionButton(
  label: string,
  action: "get" | "subscribe" | "stop-subscribe" | "offer-detail",
  key: string,
  disabled: boolean,
): HTMLButtonElement {
  const button = document.createElement("button");
  button.type = "button";
  button.textContent = label;
  button.dataset.action = action;
  button.dataset.offerKey = encodeOfferKey(key);
  button.disabled = disabled;
  return button;
}

function renderEvents(): void {
  els.eventLog.replaceChildren();
  if (state.events.length === 0) {
    const empty = document.createElement("div");
    empty.className = "event-row";
    empty.textContent = "No events";
    els.eventLog.append(empty);
    return;
  }
  for (const event of state.events) {
    const row = document.createElement("div");
    row.className = `event-row ${event.level}`;
    const time = document.createElement("span");
    time.textContent = event.at.toLocaleTimeString();
    const message = document.createElement("strong");
    message.textContent = event.message;
    const detail = document.createElement("span");
    detail.textContent = event.detail ?? "";
    row.append(time, message, detail);
    els.eventLog.append(row);
  }
}

function renderToasts(): void {
  placeToastRegion();
  els.toastRegion.replaceChildren();
  els.toastRegion.hidden = state.toasts.length === 0;
  for (const toast of state.toasts) {
    const item = document.createElement("section");
    item.className = "toast error";
    item.setAttribute("role", "alert");

    const content = document.createElement("div");
    content.className = "toast-content";

    const heading = document.createElement("div");
    heading.className = "toast-heading";
    const title = document.createElement("strong");
    title.textContent = toast.message;
    const time = document.createElement("span");
    time.textContent = toast.at.toLocaleTimeString();
    heading.append(title, time);
    content.append(heading);

    if (toast.detail) {
      const detail = document.createElement("p");
      detail.textContent = toast.detail;
      content.append(detail);
    }

    const dismiss = document.createElement("button");
    dismiss.type = "button";
    dismiss.dataset.toastId = toast.id.toString();
    dismiss.textContent = "Dismiss";

    item.append(content, dismiss);
    els.toastRegion.append(item);
  }
}

function offerRuntime(key: string): OfferRuntimeState {
  let runtime = state.offerStates.get(key);
  if (!runtime) {
    runtime = {
      status: "idle",
      snapshots: 0,
      frames: 0,
      totalBytes: 0,
      streamFrameBase: 0,
      getting: false,
      subscribing: false,
      stopping: false,
    };
    state.offerStates.set(key, runtime);
  }
  return runtime;
}

function offerByKey(key: string): OfferSummary | undefined {
  return state.offers.find((offer) => offerKey(offer) === key);
}

function isOfferPeerConnected(offer: OfferSummary): boolean {
  if (offer.peerId === state.peer?.peerId) {
    return true;
  }
  return state.peers.find((peer) => peer.peerId === offer.peerId)?.connected ?? false;
}

function offerStatusText(offer: OfferSummary, runtime: OfferRuntimeState): string {
  const isLocal = offer.peerId === state.peer?.peerId;
  if (!isLocal && !isOfferPeerConnected(offer)) {
    return "disconnected";
  }
  return isLocal && runtime.status === "idle" ? "published" : runtime.status;
}

function offerStatusClass(offer: OfferSummary, runtime: OfferRuntimeState): string {
  const isLocal = offer.peerId === state.peer?.peerId;
  if (!isLocal && !isOfferPeerConnected(offer)) {
    return "error";
  }
  return statusClass(runtime);
}

function bootstrapRecordsEventDetail(records: AukiBrowserBootstrapRecord[]): string {
  return records.map(bootstrapRecordEventDetail).join(" ; ");
}

function bootstrapRecordEventDetail(record: AukiBrowserBootstrapRecord): string {
  return [
    peerEventDetail(record.peerId),
    `addresses=${record.bootstrapAddresses.length}`,
    `transports=${transportSummary(record.bootstrapAddresses)}`,
    `relay_servers=${record.relayServerAddresses.length}`,
  ].join(" ");
}

function connectedPeerEventDetail(records: AukiBrowserBootstrapRecord[]): string {
  return records
    .map(
      (record) =>
        `${peerEventDetail(record.peerId)} connected=true active=${activePathEventDetail(record.peerId)}`,
    )
    .join(" ; ");
}

function peerListEventDetail(peers: PeerSummary[]): string {
  if (peers.length === 0) {
    return "peers=0";
  }
  return peers
    .map(
      (peer) =>
        `${peerEventDetail(peer.peerId)} dial=${peer.dialAddresses.length} observed=${peer.observedAddresses.length} transports=${transportSummary(uniqueStrings([...peer.dialAddresses, ...peer.observedAddresses]))} active=${connectionPathSummary(peer.connectionPaths)}`,
    )
    .join(" ; ");
}

function offerCatalogEventDetail(offers: OfferSummary[], peers: PeerSummary[]): string {
  const counts = new Map<string, number>();
  for (const offer of offers) {
    counts.set(offer.peerId, (counts.get(offer.peerId) ?? 0) + 1);
  }
  const byPeer = Array.from(counts.entries())
    .map(([peerId, count]) => `${shortId(peerId, 12)}=${count}`)
    .join(", ");
  return [
    `total=${offers.length}`,
    `remote_peers=${peers.length}`,
    byPeer ? `by_peer=${byPeer}` : undefined,
  ]
    .filter(Boolean)
    .join(" ");
}

function peerEventDetail(peerId: string): string {
  return `peer=${shortId(peerId, 12)}`;
}

function offerEventDetail(offer: OfferSummary): string {
  return [
    peerEventDetail(offer.peerId),
    `domain=${shortId(offer.domainId, 10)}`,
    `offer=${offer.offerId}`,
    `payload=${offer.payloadType ?? "unknown"}`,
  ].join(" ");
}

function addressEventDetail(address: string): string {
  return `transport=${transportSummary([address])} address=${address}`;
}

function activePathEventDetail(peerId: string): string {
  const peer = state.peers.find((candidate) => candidate.peerId === peerId);
  if (!peer || peer.connectionPaths.length === 0) {
    return "none";
  }
  return peer.connectionPaths.map(connectionPathEventDetail).join(",");
}

function connectionPathEventDetail(path: PeerSummary["connectionPaths"][number]): string {
  const rtt = path.rttMs === undefined ? "" : ` rtt=${Math.round(path.rttMs)}ms`;
  return `${formatTransportName(path.transport)}:${connectionPathKind(path)}:${path.direction}:${path.status}${rtt}`;
}

function recordEvent(level: "info" | "error", message: string, detail?: string): void {
  const at = new Date();
  state.events.unshift({ at, level, message, detail });
  state.events = state.events.slice(0, 80);
  if (level === "error") {
    pushErrorToast(at, message, detail);
  }
}

function pushErrorToast(at: Date, message: string, detail?: string): void {
  const id = state.nextToastId;
  state.nextToastId += 1;
  const toast: ToastEntry = {
    id,
    at,
    message,
    detail,
    timeout: window.setTimeout(() => {
      dismissToast(id);
    }, ERROR_TOAST_TIMEOUT_MS),
  };
  state.toasts.unshift(toast);
  while (state.toasts.length > MAX_ERROR_TOASTS) {
    const stale = state.toasts.pop();
    if (stale) {
      window.clearTimeout(stale.timeout);
    }
  }
  renderToasts();
}

function dismissToast(id: number): void {
  const toast = state.toasts.find((candidate) => candidate.id === id);
  if (toast) {
    window.clearTimeout(toast.timeout);
  }
  state.toasts = state.toasts.filter((candidate) => candidate.id !== id);
  renderToasts();
}

function placeToastRegion(): void {
  const dialogs = [
    els.offerDetailDialog,
    els.peerDetailDialog,
    els.addPeerDialog,
    els.diagnosticsDialog,
  ];
  let host: HTMLElement = document.body;
  for (let index = dialogs.length - 1; index >= 0; index -= 1) {
    const dialog = dialogs[index];
    if (dialog.open) {
      host = dialog;
      break;
    }
  }
  if (els.toastRegion.parentElement !== host) {
    host.append(els.toastRegion);
  }
}

function handlePeerTrace(event: AukiBrowserPeerTraceEvent): void {
  const level = event.phase === "failed" ? "error" : "info";
  const detail = [
    `attempt=${event.attempt}`,
    event.nextAttempt ? `next=${event.nextAttempt}` : undefined,
    `protocol=${event.protocol}`,
    `peer=${shortId(event.peerId, 12)}`,
    event.domainId && event.offerId ? `offer=${event.domainId}/${event.offerId}` : undefined,
    event.retryable === undefined ? undefined : `retryable=${event.retryable}`,
    event.error ? `error=${event.error}` : undefined,
  ]
    .filter(Boolean)
    .join(" ");
  recordEvent(level, `P2P ${event.operation} ${event.phase}`, detail);
  console.debug(`[auki-p2p-browser] ${event.operation} ${event.phase} ${detail}`, event);
  renderEvents();
}

async function stopSubscriptionTransport(
  subscription: AukiPreviewSubscription | undefined,
  abortController: AbortController | undefined,
  reason: string,
): Promise<void> {
  if (!subscription) {
    abortController?.abort(new Error(reason));
    return;
  }
  await withTimeout(
    subscription.stop(),
    SUBSCRIPTION_STOP_TIMEOUT_MS,
    `Subscribe stop timed out after ${SUBSCRIPTION_STOP_TIMEOUT_MS}ms`,
  );
}

async function withTimeout<T>(
  promise: Promise<T>,
  timeoutMs: number,
  message: string,
): Promise<T> {
  let timeout: ReturnType<typeof setTimeout> | undefined;
  try {
    return await Promise.race([
      promise,
      new Promise<never>((_, reject) => {
        timeout = setTimeout(() => reject(new Error(message)), timeoutMs);
      }),
    ]);
  } finally {
    if (timeout) {
      clearTimeout(timeout);
    }
  }
}

async function stopGeneratedPreview(): Promise<void> {
  const publication = state.localPreview;
  if (!publication) {
    return;
  }
  window.clearInterval(publication.timer);
  publication.source.close();
  await publication.handle.stop();
  state.localPreview = undefined;
}

async function publishGeneratedFrame(
  publication: LocalPreviewPublication,
  peerId: string,
): Promise<void> {
  const sequence = publication.nextSequence;
  publication.nextSequence += 1;
  const frame = await generatedPreviewFrame(publication.canvas, peerId, sequence);
  if (!publication.source.publish(frame)) {
    window.clearInterval(publication.timer);
    return;
  }
  renderLocalGeneratedFrame(publication, frame);
}

async function generatedPreviewFrame(
  canvas: HTMLCanvasElement,
  peerId: string,
  sequence: number,
): Promise<PublishedByteFrame> {
  const context = canvas.getContext("2d");
  if (!context) {
    throw new Error("Canvas 2D context unavailable");
  }
  const width = canvas.width;
  const height = canvas.height;
  const hue = stableHue(peerId);
  const shift = sequence % width;
  const gradient = context.createLinearGradient(0, 0, width, height);
  gradient.addColorStop(0, `hsl(${hue}, 72%, 36%)`);
  gradient.addColorStop(1, `hsl(${(hue + 78) % 360}, 76%, 48%)`);
  context.fillStyle = gradient;
  context.fillRect(0, 0, width, height);
  context.fillStyle = "rgba(0, 0, 0, 0.24)";
  context.fillRect((shift * 3) % width, 0, width / 4, height);
  context.fillStyle = "rgba(255, 255, 255, 0.62)";
  context.beginPath();
  context.arc((shift * 5) % width, height / 2, 26, 0, Math.PI * 2);
  context.fill();
  context.fillStyle = "rgba(15, 15, 15, 0.76)";
  context.font = "600 22px system-ui, sans-serif";
  context.fillText(sequence.toString().padStart(5, "0"), 16, height - 18);

  return {
    bytes: await canvasJpegBytes(canvas),
    sequence,
    generatedAt: new Date().toISOString(),
  };
}

function renderLocalGeneratedFrame(
  publication: LocalPreviewPublication,
  frame: PublishedByteFrame,
): void {
  const peer = state.peer;
  if (!peer) {
    return;
  }
  const key = `${peer.peerId}\u0000${publication.domain.domainId}\u0000${publication.offerId}`;
  const runtime = offerRuntime(key);
  const preview: PreviewFrame = {
    message: {
      type: "auki.spatial_message.v1",
      domain_id: publication.domain.domainId,
      offer_id: publication.offerId,
      payload: {},
    },
    bytes: frame.bytes,
    sequence: frame.sequence === undefined ? undefined : frame.sequence.toString(),
    generatedAt: frame.generatedAt,
  };
  const bytes = renderOfferFrame(key, runtime, preview);
  runtime.frames += 1;
  runtime.totalBytes += bytes;
  runtime.status = "published";
  updateStreamCardRuntime(key, runtime);
  updateOpenOfferDetail(key);
  renderLiveStats();
}

async function canvasJpegBytes(canvas: HTMLCanvasElement): Promise<Uint8Array> {
  const blob = await new Promise<Blob>((resolve, reject) => {
    canvas.toBlob(
      (value) => {
        if (value) {
          resolve(value);
        } else {
          reject(new Error("Failed to encode generated preview frame"));
        }
      },
      "image/jpeg",
      0.82,
    );
  });
  return new Uint8Array(await blob.arrayBuffer());
}

function stableHue(value: string): number {
  let hash = 0;
  for (let index = 0; index < value.length; index += 1) {
    hash = (hash * 31 + value.charCodeAt(index)) % 360;
  }
  return hash;
}

async function copyText(text: string): Promise<void> {
  if (!navigator.clipboard?.writeText) {
    throw new Error("Clipboard API unavailable");
  }
  await navigator.clipboard.writeText(text);
}

function offersByPeer(offers: OfferSummary[]): Map<string, OfferSummary[]> {
  const byPeer = new Map<string, OfferSummary[]>();
  for (const offer of offers) {
    const peerOffers = byPeer.get(offer.peerId) ?? [];
    peerOffers.push(offer);
    byPeer.set(offer.peerId, peerOffers);
  }
  return byPeer;
}

function aggregateRuntimeStats(): {
  snapshots: number;
  frames: number;
  totalBytes: number;
  activeStreams: number;
  rate: number;
  lastPayloadBytes?: number;
  lastSequence?: string;
  lastFrameAt?: Date;
} {
  const totals = {
    snapshots: 0,
    frames: 0,
    totalBytes: 0,
    activeStreams: 0,
    rate: 0,
    lastPayloadBytes: undefined as number | undefined,
    lastSequence: undefined as string | undefined,
    lastFrameAt: undefined as Date | undefined,
  };
  for (const runtime of state.offerStates.values()) {
    totals.snapshots += runtime.snapshots;
    totals.frames += runtime.frames;
    totals.totalBytes += runtime.totalBytes;
    totals.rate += streamRate(runtime);
    if (runtime.subscription || runtime.subscribing) {
      totals.activeStreams += 1;
    }
    if (
      runtime.lastFrameAt &&
      (!totals.lastFrameAt || runtime.lastFrameAt > totals.lastFrameAt)
    ) {
      totals.lastFrameAt = runtime.lastFrameAt;
      totals.lastPayloadBytes = runtime.lastPayloadBytes;
      totals.lastSequence = runtime.lastSequence;
    }
  }
  return totals;
}

function offerKey(offer: OfferSummary): string {
  return `${offer.peerId}\u0000${offer.domainId}\u0000${offer.offerId}`;
}

function encodeOfferKey(key: string): string {
  return encodeURIComponent(key);
}

function encodeAddress(address: string): string {
  return encodeURIComponent(address);
}

function decodeAddress(value: string | undefined): string | undefined {
  if (!value) {
    return undefined;
  }
  try {
    return decodeURIComponent(value);
  } catch {
    return undefined;
  }
}

function decodeOfferKey(value: string | undefined): string | undefined {
  if (!value) {
    return undefined;
  }
  try {
    return decodeURIComponent(value);
  } catch {
    return undefined;
  }
}

function streamRate(runtime: OfferRuntimeState): number {
  if (!runtime.subscription || runtime.stopping || !runtime.streamStartedAt) {
    return 0;
  }
  const seconds = (Date.now() - runtime.streamStartedAt.getTime()) / 1000;
  const streamFrames = runtime.frames - runtime.streamFrameBase;
  return seconds > 0 ? streamFrames / seconds : 0;
}

function statusClass(runtime: OfferRuntimeState): string {
  if (runtime.lastError || runtime.status === "error") {
    return "error";
  }
  if (runtime.subscription || runtime.status === "streaming") {
    return "live";
  }
  if (runtime.subscribing || runtime.getting || runtime.stopping) {
    return "busy";
  }
  return "idle";
}

function formatBytes(value: number): string {
  if (value < 1024) {
    return `${value} B`;
  }
  if (value < 1024 * 1024) {
    return `${(value / 1024).toFixed(1)} KiB`;
  }
  return `${(value / (1024 * 1024)).toFixed(1)} MiB`;
}

function transportSummary(addresses: readonly string[]): string {
  const transports = new Set<string>();
  for (const address of addresses) {
    if (address.includes("/webrtc-direct")) {
      transports.add("webrtc-direct");
    } else if (address.includes("/webrtc")) {
      transports.add("webrtc");
    } else if (address.includes("/p2p-circuit")) {
      transports.add("relay");
    } else if (address.includes("/ws")) {
      transports.add("websocket");
    } else {
      transports.add("other");
    }
  }
  return transports.size === 0 ? "none" : Array.from(transports).join(", ");
}

function connectionPathSummary(paths: PeerSummary["connectionPaths"]): string {
  if (paths.length === 0) {
    return "No active path";
  }
  return paths
    .map((path) => {
      const relay = path.relayInvolved ? "via relay" : "direct";
      return `${formatTransportName(path.transport)} ${relay}`;
    })
    .join(", ");
}

function connectionPathKind(path: PeerSummary["connectionPaths"][number]): string {
  if (path.relayInvolved) {
    return path.direct ? "direct via relay" : "relayed";
  }
  return path.direct ? "direct" : "limited";
}

function formatTransportName(value: string): string {
  return value.replaceAll("_", "-");
}

function bootstrapAddressList(record: AukiBrowserBootstrapRecord | undefined): string[] {
  if (!record) {
    return [];
  }
  return uniqueStrings([
    ...record.bootstrapAddresses,
    ...record.directAddresses,
    ...record.webrtcDirectAddresses,
    ...record.relayServerAddresses,
    ...record.relayAddresses,
  ]);
}

function uniqueStrings(values: readonly string[]): string[] {
  return Array.from(new Set(values));
}

function errorMessage(error: unknown): string {
  if (error instanceof AggregateError) {
    const messages = error.errors.map(errorMessage).filter((message) => message.length > 0);
    return messages.length > 0 ? messages.join("; ") : error.message;
  }
  return error instanceof Error ? error.message : String(error);
}

function element<T extends HTMLElement = HTMLElement>(id: string): T {
  const value = document.getElementById(id);
  if (!value) {
    throw new Error(`Missing element #${id}`);
  }
  return value as T;
}
