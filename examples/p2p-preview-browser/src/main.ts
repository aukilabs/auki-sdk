import {
  type AukiBrowserBootstrapRecord,
  type AukiBrowserPeer,
  type AukiBrowserPeerTraceEvent,
  type AukiPreviewSubscription,
  type OfferSummary,
  type PeerSummary,
  type PreviewFrame,
  createAukiPreviewBrowserSession,
  getPreviewSnapshot,
  openPreviewSubscription,
} from "@aukilabs/auki-p2p-browser";
import { canRequestSnapshot, offerLabel, parseBootstrapText, shortId } from "./app";
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

type AppState = {
  peer?: AukiBrowserPeer;
  bootstraps: AukiBrowserBootstrapRecord[];
  peers: PeerSummary[];
  offers: OfferSummary[];
  offerStates: Map<string, OfferRuntimeState>;
  events: EventLogEntry[];
  status: string;
  lastError?: string;
  busy: boolean;
  nextSubscriptionToken: number;
};

const SUBSCRIPTION_STOP_TIMEOUT_MS = 2_500;

const state: AppState = {
  bootstraps: [],
  peers: [],
  offers: [],
  offerStates: new Map(),
  events: [],
  status: "Idle",
  busy: false,
  nextSubscriptionToken: 0,
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
  peersTable: element<HTMLTableSectionElement>("peers-table"),
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
  state.bootstraps = [];
  render();
});
els.bootstrapFile.addEventListener("change", () => {
  void loadBootstrapFile();
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
  }
  return true;
}

render();

async function connect(): Promise<void> {
  await runShortAction("Connecting", async () => {
    const bootstraps = parseBootstrapText(els.bootstrapInput.value.trim());
    await stopPeer();
    const session = await createAukiPreviewBrowserSession({
      bootstrap: bootstraps,
      label: "p2p-preview-browser",
      trace: handlePeerTrace,
    });
    const peer = session.peer;
    recordEvent("info", "Browser peer started", shortId(peer.peerId, 12));
    recordEvent("info", "Connected bootstrap peers", bootstraps.length.toString());
    state.peer = peer;
    state.bootstraps = session.bootstraps;
    state.peers = session.peers;
    state.offers = session.offers;
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
  recordEvent("info", "Get requested", offerLabel(offer));
  render();

  try {
    const frame = await getPreviewSnapshot(peer, offer);
    const bytes = renderOfferFrame(key, runtime, frame);
    runtime.snapshots += 1;
    runtime.totalBytes += bytes;
    runtime.status = "snapshot ok";
    state.status = "Snapshot received";
    recordEvent(
      "info",
      "Get snapshot received",
      `${offerLabel(offer)} ${bytes} B in ${Math.round(performance.now() - startedAt)} ms`,
    );
  } catch (error) {
    const message = errorMessage(error);
    runtime.status = "error";
    runtime.lastError = message;
    state.lastError = message;
    state.status = "Error";
    recordEvent("error", "Get failed", `${offerLabel(offer)} ${message}`);
  } finally {
    runtime.getting = false;
    render();
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
  recordEvent("info", "Subscribe requested", offerLabel(offer));
  render();

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
    render();

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
      renderLiveStats();
    }

    if (runtime.token === token) {
      clearRuntimeSubscription(runtime);
      runtime.status = "complete";
      state.status = "Subscription complete";
      recordEvent("info", "Subscribe complete", offerLabel(offer));
      render();
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
    recordEvent("error", "Subscribe failed", `${offerLabel(offer)} ${message}`);
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
  recordEvent("info", "Subscribe stopping", reason);
  render();

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
    render();
  }
}

async function stopPeer(): Promise<void> {
  const stops = Array.from(state.offerStates.entries()).map(([key, runtime]) =>
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
  state.lastError = undefined;
  state.nextSubscriptionToken += 1;
}

async function loadBootstrapFile(): Promise<void> {
  const file = els.bootstrapFile.files?.[0];
  if (!file) {
    return;
  }
  els.bootstrapInput.value = await file.text();
  try {
    state.bootstraps = parseBootstrapText(els.bootstrapInput.value.trim());
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
  renderPeers(state.peers);
  renderStreams(state.offers);
  renderEvents();
}

function renderLiveStats(): void {
  const totals = aggregateRuntimeStats();
  els.connectionStatus.textContent = state.status;
  els.bootstrapPeer.textContent =
    state.bootstraps.length === 0
      ? "None"
      : state.bootstraps.map((record) => shortId(record.peerId, 6)).join(", ");
  els.bootstrapDirect.textContent = addressCount(
    state.bootstraps.flatMap((record) => record.directAddresses),
  );
  els.bootstrapWebrtc.textContent = addressCount(
    state.bootstraps.flatMap((record) => record.webrtcDirectAddresses),
  );
  els.bootstrapRelay.textContent = addressCount(
    state.bootstraps.flatMap((record) => record.relayServerAddresses),
  );
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
  els.peerCount.textContent = state.peers.length.toString();
  els.offerCount.textContent = state.offers.length.toString();
  els.lastError.textContent = state.lastError ?? "None";
  els.streamSummary.textContent =
    state.offers.length === 0
      ? "No offers"
      : `${totals.activeStreams} active / ${state.offers.length} offer(s)`;
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

function renderStreams(offers: OfferSummary[]): void {
  els.streamsGrid.replaceChildren();
  if (offers.length === 0) {
    const empty = document.createElement("div");
    empty.className = "empty-streams";
    empty.textContent = "No preview offers loaded";
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

function streamCard(offer: OfferSummary): HTMLElement {
  const key = offerKey(offer);
  const runtime = offerRuntime(key);
  const card = document.createElement("article");
  card.className = "stream-card";
  card.dataset.offerCard = encodeOfferKey(key);
  if (runtime.subscription || runtime.subscribing) {
    card.classList.add("streaming");
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
  status.className = `status-pill ${statusClass(runtime)}`;
  status.dataset.role = "status";
  status.textContent = runtime.status;
  header.append(title, status);

  const meta = document.createElement("div");
  meta.className = "stream-meta";
  meta.append(
    metric("Domain", shortId(offer.domainId, 7)),
    metric("Payload", offer.payloadType ?? "unknown"),
    metric("Access", offer.accessModes.join(", ")),
  );

  const stats = document.createElement("div");
  stats.className = "stream-stats";
  stats.append(
    metric("Snapshots", runtime.snapshots.toString(), "snapshots"),
    metric("Frames", runtime.frames.toString(), "frames"),
    metric("Rate", `${streamRate(runtime).toFixed(1)} fps`, "rate"),
    metric("Bytes", formatBytes(runtime.totalBytes), "bytes"),
    metric(
      "Payload",
      runtime.lastPayloadBytes === undefined ? "None" : formatBytes(runtime.lastPayloadBytes),
      "payload",
    ),
    metric("Sequence", runtime.lastSequence ?? "None", "sequence"),
    metric(
      "Last Frame",
      runtime.lastFrameAt ? runtime.lastFrameAt.toLocaleTimeString() : "Never",
      "last-frame",
    ),
  );

  const actions = document.createElement("div");
  actions.className = "row-actions";
  if (offer.accessModes.includes("get")) {
    actions.append(
      actionButton(
        runtime.getting ? "Getting" : "Get",
        "get",
        key,
        !canRequestSnapshot(Boolean(state.peer), runtime),
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
      !state.peer || runtime.stopping || runtime.getting,
    );
    if (runtime.stopping || runtime.subscribing) {
      button.classList.add("loading");
      button.setAttribute("aria-busy", "true");
    }
    actions.append(button);
  }

  if (runtime.lastError) {
    const error = document.createElement("div");
    error.className = "stream-error";
    error.textContent = runtime.lastError;
    body.append(header, meta, stats, actions, error);
  } else {
    body.append(header, meta, stats, actions);
  }
  card.append(frame, body);
  return card;
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
  if (!card) {
    return;
  }
  card.classList.toggle("streaming", Boolean(runtime.subscription || runtime.subscribing));
  const status = card.querySelector<HTMLElement>('[data-role="status"]');
  if (status) {
    status.className = `status-pill ${statusClass(runtime)}`;
    status.textContent = runtime.status;
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
  action: "get" | "subscribe" | "stop-subscribe",
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

function recordEvent(level: "info" | "error", message: string, detail?: string): void {
  state.events.unshift({ at: new Date(), level, message, detail });
  state.events = state.events.slice(0, 80);
}

function handlePeerTrace(event: AukiBrowserPeerTraceEvent): void {
  const level = event.phase === "failed" ? "error" : "info";
  const detail = [
    `attempt=${event.attempt}`,
    event.nextAttempt ? `next=${event.nextAttempt}` : undefined,
    `peer=${shortId(event.peerId, 12)}`,
    event.domainId && event.offerId ? `offer=${event.domainId}/${event.offerId}` : undefined,
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
  const value = document.getElementById(id);
  if (!value) {
    throw new Error(`Missing element #${id}`);
  }
  return value as T;
}
