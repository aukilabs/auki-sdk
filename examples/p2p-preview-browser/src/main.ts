import {
  type AukiBrowserBootstrapRecord,
  type AukiBrowserPeer,
  type AukiPreviewSubscription,
  type OfferSummary,
  type PeerSummary,
  type PreviewFrame,
  createAukiPreviewBrowserSession,
  getPreviewSnapshot,
  openPreviewSubscription,
} from "@aukilabs/auki-p2p-browser";
import { offerLabel, parseBootstrapText, shortId } from "./app";
import "./styles.css";

type OfferRuntimeState = {
  status: string;
  snapshots: number;
  frames: number;
  lastError?: string;
};

type EventLogEntry = {
  at: Date;
  level: "info" | "error";
  message: string;
  detail?: string;
};

type AppState = {
  peer?: AukiBrowserPeer;
  bootstrap?: AukiBrowserBootstrapRecord;
  peers: PeerSummary[];
  offers: OfferSummary[];
  selectedOffer?: OfferSummary;
  offerStates: Map<string, OfferRuntimeState>;
  events: EventLogEntry[];
  snapshotsReceived: number;
  framesReceived: number;
  totalBytesReceived: number;
  lastPayloadBytes?: number;
  lastSequence?: string;
  lastFrameAt?: Date;
  streamStartedAt?: Date;
  streamFrameBase: number;
  activeSubscriptionKey?: string;
  stoppingSubscriptionKey?: string;
  activeSubscription?: AukiPreviewSubscription;
  subscriptionAbort?: AbortController;
  subscriptionStop?: Promise<void>;
  status: string;
  lastError?: string;
  busy: boolean;
  subscriptionToken: number;
  previewUrl?: string;
};

const SUBSCRIPTION_STOP_TIMEOUT_MS = 2_500;

const state: AppState = {
  peers: [],
  offers: [],
  offerStates: new Map(),
  events: [],
  snapshotsReceived: 0,
  framesReceived: 0,
  totalBytesReceived: 0,
  streamFrameBase: 0,
  status: "Idle",
  busy: false,
  subscriptionToken: 0,
};

const els = {
  bootstrapInput: element<HTMLTextAreaElement>("bootstrap-input"),
  bootstrapFile: element<HTMLInputElement>("bootstrap-file"),
  connectButton: element<HTMLButtonElement>("connect-button"),
  stopButton: element<HTMLButtonElement>("stop-button"),
  clearButton: element<HTMLButtonElement>("clear-button"),
  connectionStatus: element("connection-status"),
  bootstrapPeer: element("bootstrap-peer"),
  bootstrapDirect: element("bootstrap-direct"),
  bootstrapWebrtc: element("bootstrap-webrtc"),
  bootstrapRelay: element("bootstrap-relay"),
  previewImage: element<HTMLImageElement>("preview-image"),
  previewEmpty: element("preview-empty"),
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
  peersTable: element<HTMLTableSectionElement>("peers-table"),
  offersTable: element<HTMLTableSectionElement>("offers-table"),
  eventLog: element("event-log"),
};

els.connectButton.addEventListener("click", () => {
  void connect();
});
els.stopButton.addEventListener("click", () => {
  void stop();
});
els.clearButton.addEventListener("click", () => {
  els.bootstrapInput.value = "";
  state.bootstrap = undefined;
  render();
});
els.bootstrapFile.addEventListener("change", () => {
  void loadBootstrapFile();
});
els.offersTable.addEventListener("click", (event) => {
  const target = event.target;
  if (!(target instanceof HTMLElement)) {
    return;
  }
  const button = target.closest<HTMLButtonElement>("button[data-action][data-offer-key]");
  if (!button) {
    return;
  }
  const offer = state.offers.find((candidate) => offerKey(candidate) === button.dataset.offerKey);
  if (!offer) {
    return;
  }
  if (button.dataset.action === "get") {
    void getOfferSnapshot(offer);
  } else if (button.dataset.action === "subscribe") {
    void subscribeToOffer(offer);
  } else if (button.dataset.action === "stop-subscribe") {
    void stopSubscription("Stopped by user");
  }
});

render();

async function connect(): Promise<void> {
  await runShortAction("Connecting", async () => {
    const bootstrap = parseBootstrapText(els.bootstrapInput.value.trim());
    await stopPeer();
    const session = await createAukiPreviewBrowserSession({
      bootstrap,
      label: "p2p-preview-browser",
    });
    const peer = session.peer;
    recordEvent("info", "Browser peer started", shortId(peer.peerId, 12));
    recordEvent("info", "Connected bootstrap peer", shortId(bootstrap.peerId, 12));
    state.peer = peer;
    state.bootstrap = session.bootstrap;
    state.peers = session.peers;
    state.offers = session.offers;
    state.selectedOffer = session.previewOffer;
    state.status =
      session.offers.length > 0 ? "Connected, offers loaded" : "Connected, no offers";
    recordEvent("info", "Offer catalog loaded", `${session.offers.length} offer(s)`);
  });
}

async function getOfferSnapshot(offer: OfferSummary): Promise<void> {
  const peer = state.peer;
  if (!peer || !offer.accessModes.includes("get")) {
    return;
  }

  const key = offerKey(offer);
  const startedAt = performance.now();
  state.selectedOffer = offer;
  state.busy = true;
  state.status = "Getting snapshot";
  state.lastError = undefined;
  setOfferStatus(key, "getting");
  render();
  try {
    recordEvent("info", "Get requested", offerLabel(offer));
    const frame = await getPreviewSnapshot(peer, offer);
    const bytes = renderPreviewFrame(frame);
    const runtime = offerRuntime(key);
    runtime.snapshots += 1;
    runtime.status = "snapshot ok";
    state.snapshotsReceived += 1;
    state.status = "Snapshot received";
    recordEvent(
      "info",
      "Get snapshot received",
      `${bytes} B in ${Math.round(performance.now() - startedAt)} ms`,
    );
  } catch (error) {
    const message = errorMessage(error);
    state.lastError = message;
    state.status = "Error";
    setOfferStatus(key, "error", message);
    recordEvent("error", "Get failed", message);
  } finally {
    state.busy = false;
    render();
  }
}

async function subscribeToOffer(offer: OfferSummary): Promise<void> {
  const peer = state.peer;
  if (!peer || !offer.accessModes.includes("subscribe")) {
    return;
  }
  if (state.activeSubscriptionKey) {
    await stopSubscription("Replaced by new subscription");
  }

  const key = offerKey(offer);
  const token = state.subscriptionToken + 1;
  const abortController = new AbortController();
  state.subscriptionToken = token;
  state.activeSubscriptionKey = key;
  state.subscriptionAbort = abortController;
  state.selectedOffer = offer;
  state.streamStartedAt = new Date();
  state.streamFrameBase = state.framesReceived;
  setOfferStatus(key, "subscribing");
  state.status = "Subscribing";
  state.lastError = undefined;
  recordEvent("info", "Subscribe requested", offerLabel(offer));
  render();

  try {
    const subscription = await openPreviewSubscription(peer, offer, {
      signal: abortController.signal,
    });
    if (token !== state.subscriptionToken) {
      await subscription.stop();
      return;
    }
    state.activeSubscription = subscription;
    setOfferStatus(key, "streaming");
    state.status = "Receiving";
    render();

    for await (const frame of subscription.frames) {
      if (token !== state.subscriptionToken) {
        break;
      }
      const bytes = renderPreviewFrame(frame);
      const runtime = offerRuntime(key);
      runtime.frames += 1;
      runtime.status = "streaming";
      state.framesReceived += 1;
      state.totalBytesReceived += bytes;
      state.status = "Receiving";
      renderLiveStats();
    }

    if (token === state.subscriptionToken) {
      state.activeSubscriptionKey = undefined;
      state.stoppingSubscriptionKey = undefined;
      state.activeSubscription = undefined;
      state.subscriptionAbort = undefined;
      state.subscriptionStop = undefined;
      setOfferStatus(key, "complete");
      state.status = "Subscription complete";
      recordEvent("info", "Subscribe complete", offerLabel(offer));
      render();
    }
  } catch (error) {
    if (token !== state.subscriptionToken) {
      return;
    }
    const message = errorMessage(error);
    state.activeSubscriptionKey = undefined;
    state.stoppingSubscriptionKey = undefined;
    state.activeSubscription = undefined;
    state.subscriptionAbort = undefined;
    state.subscriptionStop = undefined;
    state.lastError = message;
    state.status = "Error";
    setOfferStatus(key, "error", message);
    recordEvent("error", "Subscribe failed", message);
    render();
  }
}

async function stop(): Promise<void> {
  await runShortAction("Stopping", async () => {
    await stopPeer();
    state.status = "Stopped";
    recordEvent("info", "Browser peer stopped");
  });
}

async function stopSubscription(reason: string): Promise<void> {
  if (state.subscriptionStop) {
    await state.subscriptionStop;
    return;
  }
  if (!state.activeSubscriptionKey) {
    return;
  }

  const stop = stopSubscriptionOnce(reason);
  state.subscriptionStop = stop;
  try {
    await stop;
  } finally {
    if (state.subscriptionStop === stop) {
      state.subscriptionStop = undefined;
    }
  }
}

async function stopSubscriptionOnce(reason: string): Promise<void> {
  const key = state.activeSubscriptionKey;
  if (!key) {
    return;
  }
  const subscription = state.activeSubscription;
  const abortController = state.subscriptionAbort;
  const token = state.subscriptionToken + 1;
  state.subscriptionToken = token;
  state.stoppingSubscriptionKey = key;
  setOfferStatus(key, "stopping");
  state.status = "Stopping subscription";
  state.lastError = undefined;
  recordEvent("info", "Subscribe stopping", reason);
  render();

  try {
    await stopSubscriptionTransport(subscription, abortController, reason);
  } catch (error) {
    const message = errorMessage(error);
    state.lastError = message;
    recordEvent("error", "Subscribe stop failed", message);
  } finally {
    abortController?.abort(new Error(reason));
    if (state.subscriptionToken === token) {
      state.activeSubscriptionKey = undefined;
      state.stoppingSubscriptionKey = undefined;
      state.activeSubscription = undefined;
      state.subscriptionAbort = undefined;
      state.subscriptionStop = undefined;
      setOfferStatus(key, "stopped");
      state.status = "Subscription stopped";
    } else if (state.stoppingSubscriptionKey === key) {
      state.stoppingSubscriptionKey = undefined;
    }
    render();
  }
}

async function stopPeer(): Promise<void> {
  const subscription = state.activeSubscription;
  const abortController = state.subscriptionAbort;
  const stoppingKey = state.activeSubscriptionKey;
  if (stoppingKey) {
    state.stoppingSubscriptionKey = stoppingKey;
    setOfferStatus(stoppingKey, "stopping");
  }
  state.subscriptionAbort = undefined;
  state.activeSubscription = undefined;
  state.subscriptionStop = undefined;
  state.subscriptionToken += 1;
  await stopSubscriptionTransport(subscription, abortController, "Peer stopped").catch(
    () => undefined,
  );
  abortController?.abort(new Error("Peer stopped"));
  if (state.peer) {
    await state.peer.stop();
  }
  clearPreviewFrame();
  state.peer = undefined;
  state.bootstrap = undefined;
  state.peers = [];
  state.offers = [];
  state.selectedOffer = undefined;
  state.offerStates.clear();
  state.snapshotsReceived = 0;
  state.framesReceived = 0;
  state.totalBytesReceived = 0;
  state.lastPayloadBytes = undefined;
  state.lastSequence = undefined;
  state.lastFrameAt = undefined;
  state.streamStartedAt = undefined;
  state.streamFrameBase = 0;
  state.activeSubscriptionKey = undefined;
  state.stoppingSubscriptionKey = undefined;
  state.activeSubscription = undefined;
  state.subscriptionStop = undefined;
}

async function loadBootstrapFile(): Promise<void> {
  const file = els.bootstrapFile.files?.[0];
  if (!file) {
    return;
  }
  els.bootstrapInput.value = await file.text();
  try {
    state.bootstrap = parseBootstrapText(els.bootstrapInput.value.trim());
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

function renderPreviewFrame(frame: PreviewFrame): number {
  renderFrame(frame.bytes);
  state.lastPayloadBytes = frame.bytes.byteLength;
  state.lastSequence = frame.sequence;
  state.lastFrameAt = new Date();
  return frame.bytes.byteLength;
}

function renderFrame(bytes: Uint8Array): void {
  if (state.previewUrl) {
    URL.revokeObjectURL(state.previewUrl);
  }
  const frame = bytes.buffer.slice(
    bytes.byteOffset,
    bytes.byteOffset + bytes.byteLength,
  ) as ArrayBuffer;
  state.previewUrl = URL.createObjectURL(new Blob([frame], { type: "image/jpeg" }));
  els.previewImage.src = state.previewUrl;
}

function clearPreviewFrame(): void {
  if (state.previewUrl) {
    URL.revokeObjectURL(state.previewUrl);
  }
  state.previewUrl = undefined;
  els.previewImage.removeAttribute("src");
}

function render(): void {
  renderLiveStats();
  renderPeers(state.peers);
  renderOffers(state.offers);
  renderEvents();
}

function renderLiveStats(): void {
  els.connectionStatus.textContent = state.status;
  els.bootstrapPeer.textContent = state.bootstrap ? shortId(state.bootstrap.peerId, 10) : "None";
  els.bootstrapDirect.textContent = addressCount(state.bootstrap?.directAddresses);
  els.bootstrapWebrtc.textContent = addressCount(state.bootstrap?.webrtcDirectAddresses);
  els.bootstrapRelay.textContent = addressCount(state.bootstrap?.relayServerAddresses);
  els.snapshotsReceived.textContent = state.snapshotsReceived.toString();
  els.framesReceived.textContent = state.framesReceived.toString();
  els.streamRate.textContent = `${streamRate().toFixed(1)} fps`;
  els.totalBytes.textContent = formatBytes(state.totalBytesReceived);
  els.lastPayloadBytes.textContent =
    state.lastPayloadBytes === undefined ? "None" : formatBytes(state.lastPayloadBytes);
  els.lastSequence.textContent = state.lastSequence ?? "None";
  els.lastFrameAt.textContent = state.lastFrameAt
    ? state.lastFrameAt.toLocaleTimeString()
    : "Never";
  els.selectedOffer.textContent = offerLabel(state.selectedOffer);
  els.localPeer.textContent = state.peer ? shortId(state.peer.peerId, 10) : "Not started";
  els.peerCount.textContent = state.peers.length.toString();
  els.offerCount.textContent = state.offers.length.toString();
  els.lastError.textContent = state.lastError ?? "None";
  els.previewEmpty.hidden = Boolean(state.previewUrl);
  els.previewImage.hidden = !state.previewUrl;
  els.connectButton.disabled = state.busy;
  els.stopButton.disabled = state.busy || !state.peer;
}

function renderPeers(peers: PeerSummary[]): void {
  els.peersTable.replaceChildren();
  if (peers.length === 0) {
    appendEmptyRow(els.peersTable, 4);
    return;
  }
  for (const peer of peers) {
    const row = document.createElement("tr");
    appendTextCell(row, shortId(peer.peerId, 10));
    appendTextCell(row, peer.connected ? "yes" : "no");
    appendTextCell(row, peer.dialAddresses.length.toString());
    appendTextCell(row, transportSummary(peer.dialAddresses));
    els.peersTable.append(row);
  }
}

function renderOffers(offers: OfferSummary[]): void {
  els.offersTable.replaceChildren();
  if (offers.length === 0) {
    appendEmptyRow(els.offersTable, 7);
    return;
  }
  for (const offer of offers) {
    const key = offerKey(offer);
    const runtime = offerRuntime(key);
    const row = document.createElement("tr");
    if (state.selectedOffer && offerKey(state.selectedOffer) === key) {
      row.classList.add("selected");
    }
    appendTextCell(row, offer.kind ?? "unknown");
    appendTextCell(row, shortId(offer.domainId, 8));
    appendTextCell(row, offer.offerId);
    appendTextCell(row, offer.payloadType ?? "unknown");
    appendTextCell(row, offer.accessModes.join(", "));
    appendTextCell(row, runtimeStatus(runtime));
    row.append(offerActionCell(offer));
    els.offersTable.append(row);
  }
}

function offerActionCell(offer: OfferSummary): HTMLTableCellElement {
  const key = offerKey(offer);
  const cell = document.createElement("td");
  const actions = document.createElement("div");
  actions.className = "row-actions";
  const subscriptionBusy = Boolean(
    state.activeSubscriptionKey || state.stoppingSubscriptionKey,
  );

  if (offer.accessModes.includes("get")) {
    actions.append(actionButton("Get", "get", key, state.busy || !state.peer || subscriptionBusy));
  }
  if (offer.accessModes.includes("subscribe")) {
    const active = state.activeSubscriptionKey === key;
    const stopping = state.stoppingSubscriptionKey === key;
    const button = actionButton(
      stopping ? "Stopping" : active ? "Stop" : "Subscribe",
      active ? "stop-subscribe" : "subscribe",
      key,
      state.busy || !state.peer || stopping || (subscriptionBusy && !active),
    );
    if (stopping) {
      button.classList.add("loading");
      button.setAttribute("aria-busy", "true");
    }
    actions.append(button);
  }
  if (actions.childElementCount === 0) {
    actions.textContent = "None";
  }
  cell.append(actions);
  return cell;
}

function actionButton(
  label: string,
  action: "get" | "subscribe" | "stop-subscribe",
  key: string,
  disabled: boolean,
): HTMLButtonElement {
  const button = document.createElement("button");
  button.type = "button";
  button.textContent = label;
  button.dataset.action = action;
  button.dataset.offerKey = key;
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

function appendEmptyRow(table: HTMLTableSectionElement, colSpan: number): void {
  const row = document.createElement("tr");
  const cell = document.createElement("td");
  cell.colSpan = colSpan;
  cell.textContent = "None";
  row.append(cell);
  table.append(row);
}

function appendTextCell(row: HTMLTableRowElement, value: string): void {
  const cell = document.createElement("td");
  cell.textContent = value;
  row.append(cell);
}

function offerRuntime(key: string): OfferRuntimeState {
  let runtime = state.offerStates.get(key);
  if (!runtime) {
    runtime = { status: "idle", snapshots: 0, frames: 0 };
    state.offerStates.set(key, runtime);
  }
  return runtime;
}

function setOfferStatus(key: string, status: string, error?: string): void {
  const runtime = offerRuntime(key);
  runtime.status = status;
  runtime.lastError = error;
}

function runtimeStatus(runtime: OfferRuntimeState): string {
  const counts = [];
  if (runtime.snapshots > 0) {
    counts.push(`${runtime.snapshots} get`);
  }
  if (runtime.frames > 0) {
    counts.push(`${runtime.frames} frames`);
  }
  const suffix = counts.length > 0 ? ` (${counts.join(", ")})` : "";
  return runtime.lastError ? `${runtime.status}: ${runtime.lastError}` : `${runtime.status}${suffix}`;
}

function recordEvent(level: "info" | "error", message: string, detail?: string): void {
  state.events.unshift({ at: new Date(), level, message, detail });
  state.events = state.events.slice(0, 80);
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

function offerKey(offer: OfferSummary): string {
  return `${offer.peerId}\u0000${offer.domainId}\u0000${offer.offerId}`;
}

function streamRate(): number {
  if (
    !state.activeSubscriptionKey ||
    state.stoppingSubscriptionKey ||
    !state.streamStartedAt ||
    state.framesReceived === 0
  ) {
    return 0;
  }
  const seconds = (Date.now() - state.streamStartedAt.getTime()) / 1000;
  const streamFrames = state.framesReceived - state.streamFrameBase;
  return seconds > 0 ? streamFrames / seconds : 0;
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

function addressCount(addresses: readonly string[] | undefined): string {
  return (addresses?.length ?? 0).toString();
}

function transportSummary(addresses: readonly string[]): string {
  const transports = new Set<string>();
  for (const address of addresses) {
    if (address.includes("/webrtc-direct")) {
      transports.add("webrtc-direct");
    } else if (address.includes("/p2p-circuit")) {
      transports.add("relay");
    } else if (address.includes("/ws")) {
      transports.add("websocket");
    } else {
      transports.add("other");
    }
  }
  return transports.size === 0 ? "unknown" : Array.from(transports).join(", ");
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function element<T extends HTMLElement = HTMLElement>(id: string): T {
  const node = document.getElementById(id);
  if (!node) {
    throw new Error(`Missing element #${id}`);
  }
  return node as T;
}
