import {
  PREVIEW_PAYLOAD_TYPE,
  createAukiBrowserPeer,
  type AukiBrowserBootstrapRecord,
  type AukiBrowserPeer,
  type OfferSummary,
  type PeerSummary,
} from "@aukilabs/auki-p2p-browser";
import {
  findPreviewOffer,
  offerLabel,
  parseBootstrapText,
  previewFrameBytes,
  shortId,
} from "./app";
import "./styles.css";

type AppState = {
  peer?: AukiBrowserPeer;
  bootstrap?: AukiBrowserBootstrapRecord;
  peers: PeerSummary[];
  offers: OfferSummary[];
  selectedOffer?: OfferSummary;
  framesReceived: number;
  lastFrameAt?: Date;
  status: string;
  lastError?: string;
  subscriptionToken: number;
  previewUrl?: string;
};

const state: AppState = {
  peers: [],
  offers: [],
  framesReceived: 0,
  status: "Idle",
  subscriptionToken: 0,
};

const els = {
  bootstrapInput: element<HTMLTextAreaElement>("bootstrap-input"),
  bootstrapFile: element<HTMLInputElement>("bootstrap-file"),
  connectButton: element<HTMLButtonElement>("connect-button"),
  subscribeButton: element<HTMLButtonElement>("subscribe-button"),
  stopButton: element<HTMLButtonElement>("stop-button"),
  clearButton: element<HTMLButtonElement>("clear-button"),
  connectionStatus: element("connection-status"),
  previewImage: element<HTMLImageElement>("preview-image"),
  previewEmpty: element("preview-empty"),
  framesReceived: element("frames-received"),
  lastFrameAt: element("last-frame-at"),
  selectedOffer: element("selected-offer"),
  localPeer: element("local-peer"),
  peerCount: element("peer-count"),
  offerCount: element("offer-count"),
  lastError: element("last-error"),
  peersTable: element<HTMLTableSectionElement>("peers-table"),
  offersTable: element<HTMLTableSectionElement>("offers-table"),
};

els.connectButton.addEventListener("click", () => {
  void connect();
});
els.subscribeButton.addEventListener("click", () => {
  void subscribeToPreview();
});
els.stopButton.addEventListener("click", () => {
  void stop();
});
els.clearButton.addEventListener("click", () => {
  els.bootstrapInput.value = "";
});
els.bootstrapFile.addEventListener("change", () => {
  void loadBootstrapFile();
});

render();

async function connect(): Promise<void> {
  await runAction("Connecting", async () => {
    const bootstrap = parseBootstrapText(els.bootstrapInput.value.trim());
    await stopPeer();
    const peer = await createAukiBrowserPeer({
      bootstrap,
      label: "p2p-preview-browser",
    });
    await peer.connectBootstrap(bootstrap);
    const offers = await peer.listOffers();
    state.peer = peer;
    state.bootstrap = bootstrap;
    state.peers = peer.listPeers();
    state.offers = offers;
    state.selectedOffer = findPreviewOffer(offers);
    state.framesReceived = 0;
    state.lastFrameAt = undefined;
    state.status = state.selectedOffer ? "Connected" : "Connected, no preview offer";
  });
}

async function subscribeToPreview(): Promise<void> {
  const peer = state.peer;
  const offer = state.selectedOffer;
  if (!peer || !offer) {
    return;
  }

  const token = state.subscriptionToken + 1;
  state.subscriptionToken = token;
  await runAction("Subscribing", async () => {
    for await (const message of peer.subscribe({
      peerId: offer.peerId,
      domainId: offer.domainId,
      offerId: offer.offerId,
      acceptedPayloadTypes: [PREVIEW_PAYLOAD_TYPE],
      maxMessageBytes: 1_048_576,
    })) {
      if (token !== state.subscriptionToken) {
        break;
      }
      renderFrame(previewFrameBytes(message));
      state.framesReceived += 1;
      state.lastFrameAt = new Date();
      state.status = "Receiving";
      render();
    }
    if (token === state.subscriptionToken) {
      state.status = "Subscription complete";
    }
  });
}

async function stop(): Promise<void> {
  await runAction("Stopping", async () => {
    await stopPeer();
    state.status = "Stopped";
  });
}

async function stopPeer(): Promise<void> {
  state.subscriptionToken += 1;
  if (state.peer) {
    await state.peer.stop();
  }
  state.peer = undefined;
  state.peers = [];
  state.offers = [];
  state.selectedOffer = undefined;
}

async function loadBootstrapFile(): Promise<void> {
  const file = els.bootstrapFile.files?.[0];
  if (!file) {
    return;
  }
  els.bootstrapInput.value = await file.text();
}

async function runAction(label: string, action: () => Promise<void>): Promise<void> {
  setBusy(true);
  state.status = label;
  state.lastError = undefined;
  render();
  try {
    await action();
  } catch (error) {
    state.lastError = error instanceof Error ? error.message : String(error);
    state.status = "Error";
  } finally {
    setBusy(false);
    render();
  }
}

function setBusy(busy: boolean): void {
  els.connectButton.disabled = busy;
  els.subscribeButton.disabled = busy || !state.peer || !state.selectedOffer;
  els.stopButton.disabled = busy || !state.peer;
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

function render(): void {
  els.connectionStatus.textContent = state.status;
  els.framesReceived.textContent = state.framesReceived.toString();
  els.lastFrameAt.textContent = state.lastFrameAt
    ? state.lastFrameAt.toLocaleTimeString()
    : "Never";
  els.selectedOffer.textContent = offerLabel(state.selectedOffer);
  els.localPeer.textContent = state.peer ? shortId(state.peer.peerId) : "Not started";
  els.peerCount.textContent = state.peers.length.toString();
  els.offerCount.textContent = state.offers.length.toString();
  els.lastError.textContent = state.lastError ?? "None";
  els.previewEmpty.hidden = Boolean(state.previewUrl);
  els.previewImage.hidden = !state.previewUrl;
  els.subscribeButton.disabled = !state.peer || !state.selectedOffer;
  els.stopButton.disabled = !state.peer;

  renderPeers(state.peers);
  renderOffers(state.offers, state.selectedOffer);
}

function renderPeers(peers: PeerSummary[]): void {
  replaceRows(
    els.peersTable,
    peers.map((peer) => [
      shortId(peer.peerId),
      peer.connected ? "yes" : "no",
      peer.dialAddresses.length.toString(),
    ]),
  );
}

function renderOffers(offers: OfferSummary[], selected: OfferSummary | undefined): void {
  replaceRows(
    els.offersTable,
    offers.map((offer) => [
      offer.kind ?? "unknown",
      shortId(offer.domainId),
      offer.offerId,
      offer.payloadType ?? "unknown",
      selected === offer ? "selected" : "",
    ]),
  );
}

function replaceRows(table: HTMLTableSectionElement, rows: string[][]): void {
  table.replaceChildren();
  if (rows.length === 0) {
    const row = document.createElement("tr");
    const cell = document.createElement("td");
    cell.colSpan = 4;
    cell.textContent = "None";
    row.append(cell);
    table.append(row);
    return;
  }

  for (const values of rows) {
    const row = document.createElement("tr");
    let cells = values;
    if (values.at(-1) === "selected") {
      row.classList.add("selected");
      cells = values.slice(0, -1);
    }
    for (const value of cells) {
      const cell = document.createElement("td");
      cell.textContent = value;
      row.append(cell);
    }
    table.append(row);
  }
}

function element<T extends HTMLElement = HTMLElement>(id: string): T {
  const node = document.getElementById(id);
  if (!node) {
    throw new Error(`Missing element #${id}`);
  }
  return node as T;
}
