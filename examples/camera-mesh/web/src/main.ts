import init, { AukiUserSession } from "../pkg-web/auki_sdk_web.js";
import {
  CameraMesh,
  type CameraCandidate,
  type CameraPeerCard,
  type RemoteConnection,
  type RemoteFrame,
  type RemoteSnapshot,
} from "./camera-mesh.js";
import type { CaptureMode } from "./capture.js";

const get = <T extends HTMLElement>(id: string): T => {
  const element = document.getElementById(id);
  if (!element) throw new Error(`Camera Mesh UI is missing #${id}`);
  return element as T;
};
const input = (id: string): HTMLInputElement => get<HTMLInputElement>(id);
const button = (id: string): HTMLButtonElement => get<HTMLButtonElement>(id);

const loginForm = get<HTMLFormElement>("login");
const authSection = get<HTMLElement>("auth-section");
const email = input("email");
const password = input("password");
const loginButton = button("login-button");
const peerConfig = get<HTMLElement>("peer-config");
const domain = get<HTMLSelectElement>("domain");
const role = get<HTMLSelectElement>("role");
const displayName = input("display-name");
const localCard = get<HTMLElement>("local-card");
const runtimeSection = get<HTMLElement>("runtime-section");
const publisherPanel = get<HTMLElement>("publisher-panel");
const viewerPanel = get<HTMLElement>("viewer-panel");
const pendingList = get<HTMLElement>("pending-list");
const candidateSelect = get<HTMLSelectElement>("candidate");
const candidateDetails = get<HTMLElement>("candidate-details");
const manualCard = get<HTMLTextAreaElement>("manual-card");
const remoteFrame = get<HTMLImageElement>("remote-frame");
const remoteEmpty = get<HTMLElement>("remote-empty");
const metrics = get<HTMLElement>("metrics");
const pauseButton = button("pause-button");
const resumeButton = button("resume-button");
const snapshotButton = button("snapshot-button");
const snapshotImage = get<HTMLImageElement>("snapshot-image");
const snapshotStatus = get<HTMLElement>("snapshot-status");
const inspectorDetails = get<HTMLElement>("inspector-details");
const timeline = get<HTMLElement>("timeline");
const events = get<HTMLElement>("events");

let session: AukiUserSession | undefined;
let mesh: CameraMesh | undefined;
let candidates: CameraCandidate[] = [];
let pendingPeerIds: readonly string[] = [];
let remoteObjectUrl: string | undefined;
let snapshotObjectUrl: string | undefined;
let remoteStartedAt = 0;
let generation = 0;
const timelineRows: string[] = [];
const eventRows: string[] = [];

await init();
loginButton.disabled = false;
record("Rust/Wasm runtime ready");

loginForm.addEventListener("submit", (event) => {
  event.preventDefault();
  void login();
});
button("start-peer-button").addEventListener("click", () => void startPeer());
button("copy-card-button").addEventListener("click", () => void copyCard());
button("publish-button").addEventListener("click", () => void publish());
button("stop-publish-button").addEventListener("click", () => void stopPublishing());
button("discover-button").addEventListener("click", () => void discover());
button("add-card-button").addEventListener("click", addManualCard);
button("connect-button").addEventListener("click", () => void connect());
button("disconnect-button").addEventListener("click", () => void disconnect());
pauseButton.addEventListener("click", () => void pauseRemote());
resumeButton.addEventListener("click", () => void resumeRemote());
snapshotButton.addEventListener("click", () => void requestSnapshot());
button("stop-peer-button").addEventListener("click", () => void stopPeer());
candidateSelect.addEventListener("change", showCandidate);
window.addEventListener("beforeunload", () => {
  void mesh?.close();
  session?.free();
});

async function login(): Promise<void> {
  loginButton.disabled = true;
  let authenticated: AukiUserSession | undefined;
  try {
    authenticated = await AukiUserSession.loginDev(email.value, password.value);
    const accessible = await authenticated.accessibleDomains();
    const options = accessible.map((entry) => {
      try {
        const option = document.createElement("option");
        option.value = entry.id;
        option.textContent = entry.name ? `${entry.name} — ${entry.id}` : entry.id;
        return option;
      } finally {
        entry.free();
      }
    });
    if (options.length === 0) throw new Error("This User has no accessible Domains");
    domain.replaceChildren(...options);
    session = authenticated;
    authenticated = undefined;
    email.disabled = true;
    password.disabled = true;
    peerConfig.hidden = false;
    loginButton.textContent = "Authenticated";
    record("Authenticated. Choose a Domain and a camera role.");
  } catch (error) {
    loginButton.disabled = false;
    report(error);
  } finally {
    password.value = "";
    authenticated?.free();
  }
}

async function startPeer(): Promise<void> {
  const authenticated = session;
  if (!authenticated || mesh) return;
  const startButton = button("start-peer-button");
  startButton.disabled = true;
  const currentGeneration = ++generation;
  try {
    const started = await CameraMesh.start(
      authenticated,
      domain.value,
      role.value === "publisher" ? "publisher" : "viewer",
      displayName.value.trim() || defaultName(),
      {
        event: record,
        pendingChanged(peerIds) {
          if (generation !== currentGeneration) return;
          pendingPeerIds = peerIds;
          renderPending();
        },
        remoteFrame(frame) {
          if (generation !== currentGeneration) return;
          showRemoteFrame(frame);
        },
        remoteConnected(connection) {
          if (generation !== currentGeneration) return;
          showRemoteConnection(connection);
        },
        remoteSnapshot(snapshot) {
          if (generation !== currentGeneration) return;
          showSnapshot(snapshot);
        },
        remoteEnded(reason) {
          if (generation !== currentGeneration) return;
          record(reason);
          markRemoteStopped(reason);
        },
      },
    );
    if (generation !== currentGeneration) {
      await started.close();
      return;
    }
    mesh = started;
    session = undefined;
    authenticated.free();
    renderCard();
    authSection.hidden = true;
    peerConfig.hidden = true;
    runtimeSection.hidden = false;
    publisherPanel.hidden = started.role !== "publisher";
    viewerPanel.hidden = started.role !== "viewer";
    button("copy-card-button").disabled = false;
    record(`${capitalized(started.role)} peer ${shortPeer(started.peerId)} is ready`);
  } catch (error) {
    startButton.disabled = false;
    report(error);
  }
}

async function publish(): Promise<void> {
  const running = mesh;
  if (!running) return;
  const publishButton = button("publish-button");
  publishButton.disabled = true;
  try {
    const source = get<HTMLSelectElement>("capture-source").value as CaptureMode;
    await running.startPublishing(source, get<HTMLCanvasElement>("preview"));
    button("stop-publish-button").disabled = false;
    renderCard();
    record("Stream endpoint mounted; DDS will advertise its exact protocol");
  } catch (error) {
    publishButton.disabled = false;
    report(error);
  }
}

async function stopPublishing(): Promise<void> {
  const running = mesh;
  if (!running) return;
  button("stop-publish-button").disabled = true;
  try {
    await running.stopPublishing();
    button("publish-button").disabled = false;
    renderCard();
  } catch (error) {
    button("stop-publish-button").disabled = false;
    report(error);
  }
}

async function discover(): Promise<void> {
  const running = mesh;
  if (!running) return;
  const discoverButton = button("discover-button");
  discoverButton.disabled = true;
  try {
    candidates = (await running.discoverCameras()).filter(
      (candidate) => candidate.peerId !== running.peerId,
    );
    renderCandidates();
    record(`DDS returned ${candidates.length} Stream publisher(s)`);
  } catch (error) {
    report(error);
  } finally {
    discoverButton.disabled = false;
  }
}

async function connect(): Promise<void> {
  const running = mesh;
  const selected = candidates.find((candidate) => candidate.peerId === candidateSelect.value);
  if (!running || !selected) return;
  const connectButton = button("connect-button");
  connectButton.disabled = true;
  remoteStartedAt = performance.now();
  try {
    await running.view(selected);
    button("disconnect-button").disabled = false;
    remoteEmpty.textContent = "Waiting for the first frame…";
    record(`Stream accepted by ${shortPeer(selected.peerId)}`);
  } catch (error) {
    connectButton.disabled = false;
    const message = errorMessage(error);
    if (message.includes("approval_required")) {
      record("Publisher approval requested. Approve this Peer ID there, then retry.");
    } else {
      report(error);
    }
  }
}

async function disconnect(): Promise<void> {
  const running = mesh;
  if (!running) return;
  button("disconnect-button").disabled = true;
  try {
    await running.stopViewing();
    button("connect-button").disabled = candidates.length === 0;
    markRemoteStopped("Not connected");
    record("Remote Stream subscription closed");
  } catch (error) {
    button("disconnect-button").disabled = false;
    report(error);
  }
}

async function pauseRemote(): Promise<void> {
  const running = mesh;
  if (!running) return;
  pauseButton.disabled = true;
  try {
    await running.pauseRemote();
    resumeButton.disabled = false;
  } catch (error) {
    pauseButton.disabled = false;
    report(error);
  }
}

async function resumeRemote(): Promise<void> {
  const running = mesh;
  if (!running) return;
  resumeButton.disabled = true;
  try {
    await running.resumeRemote();
    pauseButton.disabled = false;
  } catch (error) {
    resumeButton.disabled = false;
    report(error);
  }
}

async function requestSnapshot(): Promise<void> {
  const running = mesh;
  if (!running) return;
  snapshotButton.disabled = true;
  snapshotStatus.textContent = "Message request in flight…";
  try {
    const requestId = await running.requestSnapshot();
    snapshotStatus.textContent = `Waiting for Blob announcement ${requestId}…`;
  } catch (error) {
    snapshotButton.disabled = false;
    snapshotStatus.textContent = `Snapshot failed: ${errorMessage(error)}`;
    report(error);
  }
}

async function stopPeer(): Promise<void> {
  const running = mesh;
  if (!running) return;
  mesh = undefined;
  generation += 1;
  button("stop-peer-button").disabled = true;
  let stopped = false;
  try {
    await running.close();
    stopped = true;
    record("Endpoints closed, relay booking released, and peer stopped");
  } catch (error) {
    report(error);
  } finally {
    clearRemoteUrl();
    clearSnapshotUrl();
    localCard.textContent = stopped ? "Peer stopped" : "Peer shutdown failed — see events";
    publisherPanel.hidden = true;
    viewerPanel.hidden = true;
  }
}

function addManualCard(): void {
  const running = mesh;
  if (!running || running.role !== "viewer") return;
  try {
    const candidate = parseManualCard(manualCard.value, running);
    candidates = [
      ...candidates.filter((existing) => existing.peerId !== candidate.peerId),
      candidate,
    ];
    renderCandidates(candidate.peerId);
    record(`Loaded exact camera route for ${shortPeer(candidate.peerId)} from a peer card`);
  } catch (error) {
    report(error);
  }
}

function renderCandidates(selectedPeerId?: string): void {
  const options = candidates.map((candidate) => {
    const option = document.createElement("option");
    option.value = candidate.peerId;
    option.textContent = `${shortPeer(candidate.peerId)} · ${candidate.expiresAt}`;
    return option;
  });
  if (options.length === 0) {
    const option = document.createElement("option");
    option.value = "";
    option.textContent = "No current camera publishers";
    options.push(option);
  }
  candidateSelect.replaceChildren(...options);
  candidateSelect.disabled = candidates.length === 0;
  button("connect-button").disabled = candidates.length === 0;
  if (selectedPeerId) candidateSelect.value = selectedPeerId;
  showCandidate();
}

function renderPending(): void {
  if (pendingPeerIds.length === 0) {
    pendingList.textContent = "No pending viewers.";
    return;
  }
  const rows = pendingPeerIds.map((peerId) => {
    const row = document.createElement("li");
    const identity = document.createElement("code");
    identity.textContent = peerId;
    const approve = document.createElement("button");
    approve.type = "button";
    approve.textContent = "Allow";
    approve.addEventListener("click", () => mesh?.approve(peerId));
    const deny = document.createElement("button");
    deny.type = "button";
    deny.className = "secondary";
    deny.textContent = "Deny";
    deny.addEventListener("click", () => mesh?.deny(peerId));
    const actions = document.createElement("span");
    actions.className = "actions";
    actions.append(approve, deny);
    row.append(identity, actions);
    return row;
  });
  pendingList.replaceChildren(...rows);
}

function showCandidate(): void {
  const selected = candidates.find((candidate) => candidate.peerId === candidateSelect.value);
  candidateDetails.textContent = selected
    ? JSON.stringify(selected, null, 2)
    : "Publish a camera in another tab, then refresh discovery.";
}

function showRemoteFrame(frame: RemoteFrame): void {
  const nextUrl = URL.createObjectURL(new Blob([frame.jpeg.slice().buffer], { type: "image/jpeg" }));
  const previousUrl = remoteObjectUrl;
  remoteObjectUrl = nextUrl;
  remoteFrame.src = nextUrl;
  if (previousUrl) URL.revokeObjectURL(previousUrl);
  remoteFrame.hidden = false;
  remoteEmpty.hidden = true;
  const elapsed = Math.max(0.001, (performance.now() - remoteStartedAt) / 1_000);
  const fps = frame.received / elapsed;
  const kibPerSecond = frame.bytes / elapsed / 1_024;
  metrics.textContent = `${frame.received} received · ${fps.toFixed(1)} fps · ${kibPerSecond.toFixed(1)} KiB/s · sequence ${frame.sequence}`;
}

function showRemoteConnection(connection: RemoteConnection): void {
  pauseButton.disabled = false;
  resumeButton.disabled = true;
  snapshotButton.disabled = false;
  inspectorDetails.textContent = stringify({
    info: connection.metadata.info,
    catalog: connection.metadata.catalog,
    registry: {
      sensor: inspectorRegistryEntry(connection.metadata.sensor),
      clock: inspectorRegistryEntry(connection.metadata.clock),
      frame: inspectorRegistryEntry(connection.metadata.frame),
    },
    stream: {
      route: connection.target.route,
      manifest: connection.streamManifest,
    },
  });
  record(
    `Info identified ${connection.metadata.info.name}; Catalog and 3 Registry entries verified`,
  );
}

function showSnapshot(snapshot: RemoteSnapshot): void {
  clearSnapshotUrl();
  snapshotObjectUrl = URL.createObjectURL(
    new Blob([snapshot.jpeg.slice().buffer], { type: "image/jpeg" }),
  );
  snapshotImage.src = snapshotObjectUrl;
  snapshotImage.hidden = false;
  snapshotStatus.textContent = `${snapshot.jpeg.byteLength} bytes · ${snapshot.sha256} · ${snapshot.relayed ? "relay" : "direct"}`;
  snapshotButton.disabled = false;
}

function markRemoteStopped(reason: string): void {
  clearRemoteUrl();
  remoteFrame.removeAttribute("src");
  remoteFrame.hidden = true;
  remoteEmpty.hidden = false;
  remoteEmpty.textContent = reason;
  metrics.textContent = "Stream idle";
  pauseButton.disabled = true;
  resumeButton.disabled = true;
  snapshotButton.disabled = true;
  button("disconnect-button").disabled = true;
  button("connect-button").disabled = candidates.length === 0;
}

function clearRemoteUrl(): void {
  if (remoteObjectUrl) URL.revokeObjectURL(remoteObjectUrl);
  remoteObjectUrl = undefined;
}

function clearSnapshotUrl(): void {
  if (snapshotObjectUrl) URL.revokeObjectURL(snapshotObjectUrl);
  snapshotObjectUrl = undefined;
  snapshotImage.removeAttribute("src");
  snapshotImage.hidden = true;
}

function renderCard(): void {
  localCard.textContent = mesh ? JSON.stringify(mesh.card(), null, 2) : "No peer running";
}

function inspectorRegistryEntry(entry: {
  kind: string;
  id: string;
  hash: string;
  canonicalJson: string;
}): unknown {
  return {
    kind: entry.kind,
    id: entry.id,
    expectedHash: entry.hash,
    recomputedHash: entry.hash,
    verified: true,
    canonical: JSON.parse(entry.canonicalJson) as unknown,
  };
}

function stringify(value: unknown): string {
  return JSON.stringify(
    value,
    (_key, field: unknown) => typeof field === "bigint" ? field.toString() : field,
    2,
  );
}

function parseManualCard(value: string, running: CameraMesh): CameraCandidate {
  let decoded: unknown;
  try {
    decoded = JSON.parse(value);
  } catch (error) {
    throw new Error(`Peer card is not valid JSON: ${errorMessage(error)}`);
  }
  if (typeof decoded !== "object" || decoded === null || Array.isArray(decoded)) {
    throw new Error("Peer card must be a JSON object");
  }
  const card = decoded as Record<string, unknown>;
  if (card["version"] !== 1) throw new Error("Peer card version must be 1");
  if (card["domainId"] !== running.domainId) {
    throw new Error("Peer card belongs to a different Domain");
  }
  const peerId = requiredString(card["peerId"], "Peer card peerId");
  if (peerId === running.peerId) throw new Error("Peer card points to this viewer");
  if (!Array.isArray(card["protocols"]) || !card["protocols"].includes(running.streamProtocol)) {
    throw new Error("Peer card does not advertise the Camera Stream protocol");
  }
  const routes = card["routes"];
  if (typeof routes !== "object" || routes === null || Array.isArray(routes)) {
    throw new Error("Peer card routes must be an object");
  }
  const wss = requiredString((routes as Record<string, unknown>)["wss"], "Peer card WSS route");
  if (!wss.split("/").includes("wss")) throw new Error("Peer card has no WSS route");
  return {
    peerId,
    routes: [wss],
    servedProtocols: card["protocols"].map((protocol) => String(protocol)),
    expiresAt: "manual peer card",
  };
}

function requiredString(value: unknown, label: string): string {
  if (typeof value !== "string" || value.length === 0) throw new Error(`${label} is missing`);
  return value;
}

async function copyCard(): Promise<void> {
  const card: CameraPeerCard | undefined = mesh?.card();
  if (!card) return;
  try {
    await navigator.clipboard.writeText(JSON.stringify(card, null, 2));
    record("Sanitized peer card copied");
  } catch (error) {
    report(error);
  }
}

function record(message: string): void {
  const stamp = new Date().toLocaleTimeString();
  timelineRows.push(`${stamp}  ${message}`);
  if (timelineRows.length > 80) timelineRows.splice(0, timelineRows.length - 80);
  timeline.textContent = timelineRows.join("\n");

  eventRows.push(message);
  if (eventRows.length > 20) eventRows.splice(0, eventRows.length - 20);
  events.textContent = eventRows.slice().reverse().join("\n");
}

function report(error: unknown): void {
  record(`ERROR · ${errorMessage(error)}`);
}

function defaultName(): string {
  return role.value === "publisher" ? "Chrome webcam" : "Chrome viewer";
}

function capitalized(value: string): string {
  return `${value.charAt(0).toUpperCase()}${value.slice(1)}`;
}

function shortPeer(peerId: string): string {
  return peerId.length <= 18 ? peerId : `${peerId.slice(0, 10)}…${peerId.slice(-6)}`;
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
