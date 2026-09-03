import init, { AukiUserSession } from "../pkg-web/auki_sdk_web.js";
import {
  CameraMesh,
  type CameraCandidate,
  type CameraPeerCard,
  type RemoteConnection,
  type RemoteFrame,
  type RemoteSnapshot,
} from "./camera-mesh.js";
import type { CaptureDiagnostics, CaptureMode } from "./capture.js";
import {
  CAMERA_QUALITY_TIERS,
  cameraQualityLabel,
  isCameraQualityTier,
  type CameraQualityTier,
} from "./profile.js";

type CameraStatus = "connecting" | "awaiting" | "live" | "ended" | "error";

interface FrameDeliverySample {
  readonly receivedAt: number;
  readonly bytes: number;
}

interface CameraTileState {
  readonly candidate: CameraCandidate;
  status: CameraStatus;
  message: string;
  name: string;
  operation: number;
  connection?: RemoteConnection;
  latestJpeg?: Uint8Array;
  readonly frameSurface: HTMLCanvasElement;
  frameUrl?: string;
  hasRenderedFrame: boolean;
  frameRevision: number;
  renderedRevision: number;
  displayGeneration: number;
  renderingFrame: boolean;
  received: number;
  bytes: number;
  frameSamples: FrameDeliverySample[];
  displaySamples: number[];
  displayedFrameAgeMs?: number;
  streamStartedAtMs?: number;
  timestampNs?: bigint;
  frozen: boolean;
  sourcePaused: boolean;
  snapshotPending: boolean;
  snapshotRequestId?: string;
  switchingQuality?: CameraQualityTier;
}

const MAX_CAMERAS = 16;
const DIAGNOSTIC_WINDOW_MS = 5_000;
const MAX_DIAGNOSTIC_SAMPLES = 512;
const COLUMN_STORAGE_KEY = "auki-camera-mesh-columns";
const mobileLayout = matchMedia("(max-width: 720px)");

const get = <T extends HTMLElement>(id: string): T => {
  const element = document.getElementById(id);
  if (!element) throw new Error(`Camera Mesh UI is missing #${id}`);
  return element as T;
};
const input = (id: string): HTMLInputElement => get<HTMLInputElement>(id);
const button = (id: string): HTMLButtonElement => get<HTMLButtonElement>(id);
const dialog = (id: string): HTMLDialogElement => get<HTMLDialogElement>(id);

const loginForm = get<HTMLFormElement>("login");
const authSection = get<HTMLElement>("auth-section");
const authError = get<HTMLElement>("auth-error");
const email = input("email");
const password = input("password");
const loginButton = button("login-button");
const peerConfig = get<HTMLElement>("peer-config");
const domain = get<HTMLSelectElement>("domain");
const role = get<HTMLSelectElement>("role");
const displayName = input("display-name");
const runtimeSection = get<HTMLElement>("runtime-section");
const viewerPanel = get<HTMLElement>("viewer-panel");
const publisherPanel = get<HTMLElement>("publisher-panel");
const viewerToolbar = get<HTMLElement>("viewer-toolbar");
const cameraGrid = get<HTMLElement>("camera-grid");
const wallStatus = get<HTMLElement>("wall-status");
const domainName = get<HTMLElement>("domain-name");
const peerStatusDot = get<HTMLElement>("peer-status-dot");
const peerIdLabel = get<HTMLElement>("peer-id-label");
const localCard = get<HTMLElement>("local-card");
const aggregateMetrics = get<HTMLElement>("metrics");
const pendingList = get<HTMLElement>("pending-list");
const publisherTitle = get<HTMLElement>("publisher-title");
const publisherBadge = get<HTMLElement>("publisher-badge");
const cameraSpec = get<HTMLElement>("camera-spec");
const previewEmpty = get<HTMLElement>("preview-empty");
const addDialog = dialog("add-camera-dialog");
const diagnosticsDialog = dialog("diagnostics-dialog");
const snapshotDialog = dialog("snapshot-dialog");
const cameraActionsDialog = dialog("camera-actions-dialog");
const cameraActionsTitle = get<HTMLElement>("camera-actions-title");
const cameraResults = get<HTMLElement>("camera-results");
const addAllCamerasButton = button("add-all-cameras-button");
const manualCard = get<HTMLTextAreaElement>("manual-card");
const addCameraError = get<HTMLElement>("add-camera-error");
const diagnosticFps = get<HTMLOutputElement>("diagnostic-fps");
const diagnosticProfile = get<HTMLOutputElement>("diagnostic-profile");
const diagnosticCaptureFps = get<HTMLOutputElement>("diagnostic-capture-fps");
const diagnosticCaptureDetail = get<HTMLElement>("diagnostic-capture-detail");
const diagnosticDisplayFps = get<HTMLOutputElement>("diagnostic-display-fps");
const diagnosticBandwidth = get<HTMLOutputElement>("diagnostic-bandwidth");
const diagnosticFrameSize = get<HTMLElement>("diagnostic-frame-size");
const diagnosticFrameAge = get<HTMLOutputElement>("diagnostic-frame-age");
const inspectorDetails = get<HTMLElement>("inspector-details");
const timeline = get<HTMLElement>("timeline");
const events = get<HTMLElement>("events");
const snapshotImage = get<HTMLImageElement>("snapshot-image");
const snapshotStatus = get<HTMLElement>("snapshot-status");
const snapshotTitle = get<HTMLElement>("snapshot-title");
const toast = get<HTMLElement>("toast");
const focusControls = get<HTMLElement>("focus-controls");
const focusLabel = get<HTMLElement>("focus-label");

let session: AukiUserSession | undefined;
let mesh: CameraMesh | undefined;
let candidates = new Map<string, CameraCandidate>();
let pendingPeerIds: readonly string[] = [];
const cameras = new Map<string, CameraTileState>();
let cameraOrder: string[] = [];
let selectedPeerId: string | undefined;
let actionPeerId: string | undefined;
let columnCount = initialColumnCount();
let generation = 0;
let snapshotObjectUrl: string | undefined;
let toastTimer: number | undefined;
let addingAllCameras = false;
const publisherDiagnostics = new Map<CameraQualityTier, CaptureDiagnostics>();
const timelineRows: string[] = [];
const eventRows: string[] = [];

await init();
loginButton.disabled = false;
record("Rust/Wasm runtime ready");
renderWall();

loginForm.addEventListener("submit", (event) => {
  event.preventDefault();
  void login();
});
button("start-peer-button").addEventListener("click", () => void startPeer());
button("copy-card-button").addEventListener("click", () => void copyCard());
button("open-diagnostics-button").addEventListener("click", () => openDiagnostics());
button("stop-peer-button").addEventListener("click", () => void stopPeer());
button("publish-button").addEventListener("click", () => void publish());
button("stop-publish-button").addEventListener("click", () => void stopPublishing());
button("add-camera-button").addEventListener("click", openAddCamera);
button("discover-button").addEventListener("click", () => void discover());
addAllCamerasButton.addEventListener("click", () => void addAllCameras());
button("add-card-button").addEventListener("click", () => void addManualCard());
button("close-add-camera-button").addEventListener("click", () => addDialog.close());
button("close-diagnostics-button").addEventListener("click", () => diagnosticsDialog.close());
button("close-snapshot-button").addEventListener("click", () => snapshotDialog.close());
button("close-camera-actions-button").addEventListener("click", () => cameraActionsDialog.close());
button("previous-camera-button").addEventListener("click", () => changeFocusedCamera(-1));
button("next-camera-button").addEventListener("click", () => changeFocusedCamera(1));

for (const control of document.querySelectorAll<HTMLButtonElement>("[data-column-count]")) {
  control.addEventListener("click", () => setColumnCount(Number(control.dataset.columnCount)));
}
mobileLayout.addEventListener("change", () => renderWall());

cameraResults.addEventListener("click", (event) => {
  const control = (event.target as Element).closest<HTMLButtonElement>("button[data-candidate-peer-id]");
  if (!control) return;
  const candidate = candidates.get(control.dataset.candidatePeerId ?? "");
  if (candidate) void addCamera(candidate);
});

cameraGrid.addEventListener("click", (event) => {
  const target = event.target as Element;
  const tile = target.closest<HTMLElement>("[data-camera-peer-id]");
  if (tile?.dataset.cameraPeerId) selectCamera(tile.dataset.cameraPeerId);
  const control = target.closest<HTMLButtonElement>("button[data-action]");
  if (!control) return;
  const action = control.dataset.action;
  const peerId = control.dataset.peerId;
  if (action === "add") {
    openAddCamera();
  } else if (peerId) {
    void handleTileAction(action ?? "", peerId);
  }
});

cameraActionsDialog.addEventListener("click", (event) => {
  const control = (event.target as Element).closest<HTMLButtonElement>("button[data-camera-action]");
  const peerId = actionPeerId;
  if (!control || !peerId) return;
  const action = control.dataset.cameraAction;
  cameraActionsDialog.close();
  if (action) void handleTileAction(action, peerId);
});

for (const candidate of [addDialog, diagnosticsDialog, snapshotDialog, cameraActionsDialog]) {
  candidate.addEventListener("click", (event) => {
    if (event.target === candidate) candidate.close();
  });
}

document.addEventListener("click", (event) => {
  const menu = document.querySelector<HTMLDetailsElement>(".session-menu");
  if (menu?.open && !menu.contains(event.target as Node)) menu.open = false;
});

window.addEventListener("beforeunload", () => {
  clearAllCameraSurfaces();
  clearSnapshotUrl();
  void mesh?.close();
  session?.free();
});

async function login(): Promise<void> {
  loginButton.disabled = true;
  clearInlineError(authError);
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
    loginForm.hidden = true;
    peerConfig.hidden = false;
    record("Authenticated. Choose a Domain and camera mode.");
  } catch (error) {
    loginButton.disabled = false;
    showInlineError(authError, errorMessage(error));
    report(error, false);
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
  clearInlineError(authError);
  const currentGeneration = ++generation;
  try {
    const started = await CameraMesh.start(
      authenticated,
      domain.value,
      role.value === "publisher" ? "publisher" : "viewer",
      displayName.value.trim() || defaultName(),
      {
        event: record,
        captureDiagnostics(diagnostics) {
          if (generation !== currentGeneration) return;
          publisherDiagnostics.set(diagnostics.quality, diagnostics);
          updateAggregateMetrics();
          renderDiagnostics();
        },
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
        snapshotExpired(requestId, peerId) {
          if (generation !== currentGeneration) return;
          const state = peerId ? cameras.get(peerId) : undefined;
          if (state) {
            state.snapshotPending = false;
            state.snapshotRequestId = undefined;
            renderWall();
          }
          snapshotStatus.textContent = `Snapshot ${requestId} timed out`;
          record(`Snapshot request ${requestId} timed out without an announcement`);
          showToast("Snapshot request timed out", true);
        },
        remoteEnded(reason, peerId) {
          if (generation !== currentGeneration) return;
          record(reason);
          if (peerId) {
            markCameraEnded(peerId, reason);
          } else {
            peerStatusDot.className = "status-dot error";
            wallStatus.textContent = "Peer failed";
            for (const state of cameras.values()) {
              state.status = "ended";
              state.message = reason;
            }
            renderWall();
          }
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
    runtimeSection.hidden = false;
    publisherPanel.hidden = started.role !== "publisher";
    viewerPanel.hidden = started.role !== "viewer";
    viewerToolbar.hidden = started.role !== "viewer";
    peerStatusDot.className = "status-dot live";
    domainName.textContent = selectedDomainName();
    peerIdLabel.textContent = started.peerId;
    cameraSpec.textContent = started.profiles
      .map((profile) => `${profile.quality} ${profile.width}×${profile.height}@${profile.rateHz}`)
      .join(" · ");
    button("copy-card-button").disabled = false;
    record(`${capitalized(started.role)} peer ${shortPeer(started.peerId)} is ready`);
    if (started.role === "viewer") {
      renderWall();
    } else {
      wallStatus.textContent = "Camera offline";
      updateAggregateMetrics();
    }
  } catch (error) {
    startButton.disabled = false;
    showInlineError(authError, errorMessage(error));
    report(error, false);
  }
}

async function publish(): Promise<void> {
  const running = mesh;
  if (!running) return;
  const publishButton = button("publish-button");
  publishButton.disabled = true;
  publisherDiagnostics.clear();
  try {
    const source = get<HTMLSelectElement>("capture-source").value as CaptureMode;
    await running.startPublishing(source, get<HTMLCanvasElement>("preview"));
    button("stop-publish-button").disabled = false;
    previewEmpty.hidden = true;
    publisherBadge.textContent = "LIVE";
    publisherBadge.classList.add("live");
    publisherTitle.textContent = running.name;
    wallStatus.textContent = "Camera live";
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
    publisherDiagnostics.clear();
    button("publish-button").disabled = false;
    previewEmpty.hidden = false;
    publisherBadge.textContent = "OFFLINE";
    publisherBadge.classList.remove("live");
    publisherTitle.textContent = "Ready to publish";
    wallStatus.textContent = "Camera offline";
    renderCard();
  } catch (error) {
    button("stop-publish-button").disabled = false;
    report(error);
  }
}

function openAddCamera(): void {
  if (!mesh || mesh.role !== "viewer") return;
  if (cameras.size >= MAX_CAMERAS) {
    showToast("The camera wall already contains 16 cameras", true);
    return;
  }
  clearInlineError(addCameraError);
  renderCandidates();
  showModal(addDialog);
  void discover();
}

async function discover(): Promise<void> {
  const running = mesh;
  if (!running || running.role !== "viewer") return;
  const discoverButton = button("discover-button");
  discoverButton.disabled = true;
  discoverButton.textContent = "Scanning…";
  clearInlineError(addCameraError);
  try {
    const discovered = (await running.discoverCameras()).filter(
      (candidate) => candidate.peerId !== running.peerId,
    );
    const next = new Map<string, CameraCandidate>();
    for (const state of cameras.values()) next.set(state.candidate.peerId, state.candidate);
    for (const candidate of discovered) next.set(candidate.peerId, candidate);
    candidates = next;
    renderCandidates();
    record(`DDS returned ${discovered.length} Stream publisher(s)`);
  } catch (error) {
    showInlineError(addCameraError, errorMessage(error));
    report(error, false);
  } finally {
    discoverButton.disabled = false;
    discoverButton.textContent = "Refresh";
  }
}

async function addManualCard(): Promise<void> {
  const running = mesh;
  if (!running || running.role !== "viewer") return;
  clearInlineError(addCameraError);
  try {
    const candidate = parseManualCard(manualCard.value, running);
    candidates.set(candidate.peerId, candidate);
    manualCard.value = "";
    record(`Loaded exact camera route for ${shortPeer(candidate.peerId)} from a peer card`);
    await addCamera(candidate);
  } catch (error) {
    showInlineError(addCameraError, errorMessage(error));
    report(error, false);
  }
}

async function addAllCameras(): Promise<void> {
  if (addingAllCameras) return;
  const additions = addableCandidates();
  if (additions.length === 0) return;

  addingAllCameras = true;
  addAllCamerasButton.disabled = true;
  addAllCamerasButton.textContent = `Adding ${additions.length}…`;
  record(
    `Burst-connecting ${additions.length} discovered camera(s) concurrently (stress path)`,
  );
  addDialog.close();
  try {
    const results = await Promise.allSettled(
      additions.map((candidate) => addCamera(candidate)),
    );
    for (const result of results) {
      if (result.status === "rejected") report(result.reason, false);
    }
  } finally {
    addingAllCameras = false;
    renderCandidates();
    const live = [...cameras.values()].filter((state) => state.status === "live").length;
    const failed = [...cameras.values()].filter(
      (state) => state.status === "error" || state.status === "ended",
    ).length;
    showToast(
      `Burst complete · ${live} live${failed ? ` · ${failed} failed` : ""}`,
      failed > 0,
    );
  }
}

function addableCandidates(): CameraCandidate[] {
  let remainingSlots = MAX_CAMERAS - cameras.size;
  return [...candidates.values()]
    .sort((left, right) => left.peerId.localeCompare(right.peerId))
    .filter((candidate) => {
      const state = cameras.get(candidate.peerId);
      if (state) return state.status !== "live" && state.status !== "connecting";
      if (remainingSlots === 0) return false;
      remainingSlots -= 1;
      return true;
    });
}

async function addCamera(candidate: CameraCandidate): Promise<void> {
  const running = mesh;
  if (!running || running.role !== "viewer") return;
  let state = cameras.get(candidate.peerId);
  const existing = state !== undefined;
  if (!state) {
    if (cameras.size >= MAX_CAMERAS) {
      showInlineError(addCameraError, "The camera wall already contains 16 cameras");
      return;
    }
    state = {
      candidate,
      status: "connecting",
      message: "Verifying camera metadata and opening the Stream…",
      name: `Camera ${shortPeer(candidate.peerId)}`,
      operation: 0,
      received: 0,
      bytes: 0,
      frameSamples: [],
      displaySamples: [],
      frameSurface: document.createElement("canvas"),
      hasRenderedFrame: false,
      frameRevision: 0,
      renderedRevision: 0,
      displayGeneration: 0,
      renderingFrame: false,
      frozen: false,
      sourcePaused: false,
      snapshotPending: false,
    };
    cameras.set(candidate.peerId, state);
    cameraOrder.push(candidate.peerId);
  }
  if (existing && (state.status === "live" || state.status === "connecting")) {
    selectedPeerId = candidate.peerId;
    renderWall();
    addDialog.close();
    return;
  }

  const operation = ++state.operation;
  state.status = "connecting";
  state.message = "Verifying camera metadata and opening the Stream…";
  resetStreamDiagnostics(state);
  state.sourcePaused = false;
  selectedPeerId = candidate.peerId;
  renderWall();
  addDialog.close();
  try {
    const connection = await running.connectCamera(candidate, preferredQualityForWall());
    if (cameras.get(candidate.peerId) !== state || state.operation !== operation) {
      await running.disconnectCamera(candidate.peerId);
      return;
    }
    state.connection = connection;
    state.name = connection.metadata.info.name || state.name;
    state.status = "live";
    state.message = "Waiting for the first frame…";
    renderWall();
    renderDiagnostics();
    record(`Stream accepted by ${shortPeer(candidate.peerId)}`);
  } catch (error) {
    if (cameras.get(candidate.peerId) !== state || state.operation !== operation) return;
    const message = errorMessage(error);
    if (message.includes("approval_required")) {
      state.status = "awaiting";
      state.message = "Approve this viewer on the camera peer, then retry.";
      record("Publisher approval requested. Approve this Peer ID there, then retry.");
    } else {
      state.status = "error";
      state.message = readableConnectionError(message);
      report(error, false);
    }
    renderWall();
  }
}

async function handleTileAction(action: string, peerId: string): Promise<void> {
  const state = cameras.get(peerId);
  if (!state) return;
  if (action === "menu") {
    openCameraActions(peerId);
  } else if (action === "retry") {
    await addCamera(state.candidate);
  } else if (action === "remove") {
    await removeCamera(peerId);
  } else if (action === "freeze") {
    state.frozen = !state.frozen;
    renderWall();
  } else if (action === "snapshot") {
    await requestSnapshot(peerId);
  } else if (action === "source-pause") {
    await setRemoteSourcePaused(peerId, true);
  } else if (action === "source-resume") {
    await setRemoteSourcePaused(peerId, false);
  } else if (action === "details") {
    openDiagnostics(peerId);
  } else if (action === "fullscreen") {
    await openFullscreen(peerId);
  } else if (action.startsWith("quality-")) {
    const quality = action.slice("quality-".length);
    if (isCameraQualityTier(quality)) await switchCameraQuality(peerId, quality);
  }
}

async function switchCameraQuality(
  peerId: string,
  quality: CameraQualityTier,
): Promise<void> {
  const running = mesh;
  const state = cameras.get(peerId);
  if (
    !running
    || !state?.connection
    || state.status !== "live"
    || state.switchingQuality !== undefined
    || state.connection.metadata.profile.quality === quality
  ) return;
  state.switchingQuality = quality;
  renderWall();
  try {
    await running.switchCameraQuality(peerId, quality);
    showToast(`${state.name} switched to ${cameraQualityLabel(quality)}`);
  } catch (error) {
    report(error);
  } finally {
    state.switchingQuality = undefined;
    renderWall();
    renderDiagnostics();
  }
}

async function removeCamera(peerId: string): Promise<void> {
  const running = mesh;
  const state = cameras.get(peerId);
  if (!running || !state) return;
  state.operation += 1;
  cameras.delete(peerId);
  cameraOrder = cameraOrder.filter((candidate) => candidate !== peerId);
  clearCameraSurface(state);
  if (selectedPeerId === peerId) selectedPeerId = cameraOrder[0];
  if (actionPeerId === peerId) actionPeerId = undefined;
  renderWall();
  try {
    await running.disconnectCamera(peerId);
    record(`Remote Stream subscription closed for ${shortPeer(peerId)}`);
  } catch (error) {
    report(error);
  }
}

async function setRemoteSourcePaused(peerId: string, paused: boolean): Promise<void> {
  const running = mesh;
  const state = cameras.get(peerId);
  if (!running || !state || state.status !== "live") return;
  try {
    if (paused) {
      await running.pauseRemote(peerId);
    } else {
      await running.resumeRemote(peerId);
    }
    state.sourcePaused = paused;
    state.message = paused ? "Camera source paused for every viewer." : "Waiting for the next frame…";
    renderWall();
  } catch (error) {
    report(error);
  }
}

async function requestSnapshot(peerId: string): Promise<void> {
  const running = mesh;
  const state = cameras.get(peerId);
  if (!running || !state || state.status !== "live" || state.snapshotPending) return;
  state.snapshotPending = true;
  state.snapshotRequestId = undefined;
  renderWall();
  try {
    const requestId = await running.requestSnapshot(peerId);
    if (state.snapshotPending) state.snapshotRequestId = requestId;
    snapshotStatus.textContent = `Waiting for Blob announcement ${requestId}…`;
  } catch (error) {
    state.snapshotPending = false;
    state.snapshotRequestId = undefined;
    renderWall();
    report(error);
  }
}

async function openFullscreen(peerId: string): Promise<void> {
  const tile = cameraElement(peerId);
  if (!tile) return;
  try {
    await tile.requestFullscreen();
  } catch (error) {
    report(error);
  }
}

function renderCandidates(): void {
  const addable = addableCandidates();
  addAllCamerasButton.disabled = addingAllCameras || addable.length === 0;
  addAllCamerasButton.textContent = addingAllCameras
    ? "Adding…"
    : addable.length > 0
      ? `Add all (${addable.length})`
      : "Add all";
  if (candidates.size === 0) {
    const empty = document.createElement("p");
    empty.className = "empty-message";
    empty.textContent = "No cameras found yet. Refresh, or use a peer card below.";
    cameraResults.replaceChildren(empty);
    return;
  }
  const rows = [...candidates.values()]
    .sort((left, right) => left.peerId.localeCompare(right.peerId))
    .map((candidate) => {
      const row = document.createElement("article");
      row.className = "camera-result";
      row.dataset.candidatePeerId = candidate.peerId;
      const identity = document.createElement("div");
      const title = document.createElement("strong");
      const state = cameras.get(candidate.peerId);
      title.textContent = state?.connection?.metadata.info.name ?? "Camera publisher";
      const peer = document.createElement("code");
      peer.textContent = `${shortPeer(candidate.peerId)} · ${formatExpiry(candidate.expiresAt)}`;
      peer.title = candidate.peerId;
      identity.append(title, peer);
      const action = document.createElement("button");
      action.type = "button";
      action.className = state ? "secondary compact" : "primary compact";
      action.dataset.candidatePeerId = candidate.peerId;
      action.disabled = addingAllCameras
        || state?.status === "live"
        || state?.status === "connecting";
      action.textContent = state?.status === "live"
        ? "On wall"
        : state?.status === "connecting"
          ? "Connecting…"
          : state
            ? "Retry"
            : "Add";
      row.append(identity, action);
      return row;
    });
  cameraResults.replaceChildren(...rows);
}

function renderWall(): void {
  if (selectedPeerId && !cameras.has(selectedPeerId)) selectedPeerId = undefined;
  selectedPeerId ??= cameraOrder[0];
  const renderedColumns = effectiveColumnCount();
  const focusMode = renderedColumns === 1;
  const visiblePeerIds = focusMode
    ? selectedPeerId ? [selectedPeerId] : []
    : [...cameraOrder];
  const visible = new Set(visiblePeerIds);
  for (const [peerId, state] of cameras) {
    if (!visible.has(peerId)) clearCameraSurface(state);
  }

  cameraGrid.dataset.columnCount = String(renderedColumns);
  cameraGrid.style.setProperty("--column-count", String(renderedColumns));
  const children: HTMLElement[] = visiblePeerIds.map((peerId) => {
    const state = cameras.get(peerId);
    return state ? renderCameraTile(state) : renderEmptyTile();
  });
  if ((!focusMode || children.length === 0) && cameras.size < MAX_CAMERAS) {
    children.push(renderEmptyTile());
  }
  cameraGrid.replaceChildren(...children);
  for (const peerId of visiblePeerIds) {
    const state = cameras.get(peerId);
    if (state) scheduleFrameDisplay(state);
  }

  for (const control of document.querySelectorAll<HTMLButtonElement>("[data-column-count]")) {
    control.setAttribute(
      "aria-pressed",
      String(Number(control.dataset.columnCount) === renderedColumns),
    );
  }
  const focusIndex = selectedPeerId ? cameraOrder.indexOf(selectedPeerId) : -1;
  focusControls.hidden = !focusMode || cameraOrder.length < 2;
  focusLabel.textContent = focusIndex >= 0
    ? `${focusIndex + 1} / ${cameraOrder.length}`
    : `0 / ${cameraOrder.length}`;
  button("previous-camera-button").disabled = cameraOrder.length < 2;
  button("next-camera-button").disabled = cameraOrder.length < 2;
  button("add-camera-button").disabled = cameras.size >= MAX_CAMERAS;
  updateWallStatus();
  updateAggregateMetrics();
}

function renderCameraTile(state: CameraTileState): HTMLElement {
  const peerId = state.candidate.peerId;
  const tile = document.createElement("article");
  tile.className = "camera-tile";
  if (selectedPeerId === peerId) tile.classList.add("selected");
  tile.dataset.cameraPeerId = peerId;
  tile.dataset.status = state.status;
  tile.dataset.frameCount = String(state.received);
  if (state.connection) tile.dataset.quality = state.connection.metadata.profile.quality;
  setTileDiagnosticAttributes(tile, state);
  tile.tabIndex = 0;
  tile.setAttribute("aria-label", `${state.name}, ${statusLabel(state)}`);

  const surface = state.frameSurface;
  surface.className = "camera-feed";
  surface.dataset.role = "remote-frame";
  surface.setAttribute("aria-label", `Live feed from ${state.name}`);
  surface.hidden = !state.hasRenderedFrame;
  surface.classList.toggle("dimmed", state.status !== "live");

  const shade = document.createElement("div");
  shade.className = "tile-shade";
  const top = document.createElement("div");
  top.className = "tile-topline";
  const identity = document.createElement("div");
  identity.className = "camera-identity";
  const number = document.createElement("span");
  number.className = "camera-number";
  number.textContent = `CAM ${String(cameraOrder.indexOf(peerId) + 1).padStart(2, "0")}`;
  const name = document.createElement("span");
  name.className = "camera-name";
  name.textContent = state.name;
  identity.append(number, name);
  const status = document.createElement("span");
  status.className = "feed-status";
  status.textContent = statusLabel(state);
  top.append(identity, status);

  const center = document.createElement("div");
  center.className = "tile-center";
  if (state.status !== "live" || !state.hasRenderedFrame) {
    const message = document.createElement("div");
    message.className = "tile-message";
    if (state.status === "connecting") {
      const spinner = document.createElement("span");
      spinner.className = "tile-spinner";
      spinner.setAttribute("aria-hidden", "true");
      message.append(spinner);
    }
    const heading = document.createElement("strong");
    heading.textContent = centerHeading(state);
    const copy = document.createElement("span");
    copy.textContent = state.message;
    message.append(heading, copy);
    if (state.status === "awaiting" || state.status === "error" || state.status === "ended") {
      const retry = document.createElement("button");
      retry.type = "button";
      retry.className = "secondary compact tile-primary-action";
      retry.dataset.action = "retry";
      retry.dataset.peerId = peerId;
      retry.textContent = "Retry";
      message.append(retry);
    }
    center.append(message);
  }

  const bottom = document.createElement("div");
  bottom.className = "tile-bottomline";
  const frameTime = document.createElement("span");
  frameTime.className = "frame-time";
  frameTime.dataset.role = "frame-time";
  frameTime.textContent = state.timestampNs ? formatFrameTime(state.timestampNs) : shortPeer(peerId);
  const streamMetrics = document.createElement("span");
  streamMetrics.className = "stream-metrics";
  streamMetrics.dataset.role = "stream-diagnostics";
  setTileDiagnostics(streamMetrics, state);
  const frameDetails = document.createElement("div");
  frameDetails.className = "frame-details";
  frameDetails.append(frameTime, streamMetrics);
  const actions = document.createElement("div");
  actions.className = "tile-actions";
  if (state.status === "live") {
    actions.append(
      tileAction("freeze", peerId, state.frozen ? "▶" : "Ⅱ", state.frozen ? "Resume local view" : "Freeze local view"),
      tileAction("snapshot", peerId, "◎", "Verified snapshot", state.snapshotPending),
      tileAction("fullscreen", peerId, "⛶", "Full screen"),
      tileAction("menu", peerId, "•••", "Camera actions", false, true),
    );
  } else {
    actions.append(
      tileAction("retry", peerId, "↻", "Retry camera"),
      tileAction("remove", peerId, "×", "Remove camera"),
      tileAction("menu", peerId, "•••", "Camera actions", false, true),
    );
  }
  bottom.append(frameDetails, actions);
  tile.append(surface, shade, top, center, bottom);
  return tile;
}

function renderEmptyTile(): HTMLElement {
  const tile = document.createElement("div");
  tile.className = "empty-tile";
  const add = document.createElement("button");
  add.type = "button";
  add.dataset.action = "add";
  add.disabled = cameras.size >= MAX_CAMERAS;
  const symbol = document.createElement("span");
  symbol.textContent = "+";
  symbol.setAttribute("aria-hidden", "true");
  const label = document.createElement("span");
  label.textContent = "Add camera";
  add.append(symbol, label);
  tile.append(add);
  return tile;
}

function tileAction(
  action: string,
  peerId: string,
  symbol: string,
  label: string,
  disabled = false,
  menu = false,
): HTMLButtonElement {
  const control = document.createElement("button");
  control.type = "button";
  control.className = menu ? "tile-action tile-menu-trigger" : "tile-action";
  control.dataset.action = action;
  control.dataset.peerId = peerId;
  control.textContent = symbol;
  control.title = label;
  control.setAttribute("aria-label", label);
  control.disabled = disabled;
  return control;
}

function openCameraActions(peerId: string): void {
  const state = cameras.get(peerId);
  if (!state) return;
  actionPeerId = peerId;
  selectedPeerId = peerId;
  cameraActionsTitle.textContent = state.name;
  for (const control of cameraActionsDialog.querySelectorAll<HTMLButtonElement>(
    "button[data-camera-action]",
  )) {
    const action = control.dataset.cameraAction;
    const liveOnly = action === "freeze"
      || action === "snapshot"
      || action === "fullscreen"
      || action === "source-pause"
      || action === "source-resume"
      || action?.startsWith("quality-") === true;
    control.hidden = liveOnly && state.status !== "live";
    if (action === "freeze") {
      control.textContent = state.frozen ? "Resume local view" : "Freeze local view";
    } else if (action === "snapshot") {
      control.disabled = state.snapshotPending;
    } else if (action === "source-pause" || action === "source-resume") {
      control.dataset.cameraAction = state.sourcePaused ? "source-resume" : "source-pause";
      control.textContent = state.sourcePaused
        ? "Resume camera for every viewer"
        : "Pause camera for every viewer";
    } else if (action === "retry") {
      control.hidden = state.status === "live" || state.status === "connecting";
    } else if (action?.startsWith("quality-")) {
      const quality = action.slice("quality-".length);
      if (!isCameraQualityTier(quality)) continue;
      const available = state.connection?.metadata.availableQualities ?? [];
      const active = state.connection?.metadata.profile.quality === quality;
      control.disabled = state.switchingQuality !== undefined
        || !available.includes(quality)
        || active;
      control.setAttribute("aria-pressed", String(active));
      control.title = available.includes(quality)
        ? active ? "Current stream quality" : `Switch to ${cameraQualityLabel(quality)}`
        : "This publisher does not offer this quality";
    }
  }
  renderDiagnostics();
  showModal(cameraActionsDialog);
}

function showRemoteFrame(frame: RemoteFrame): void {
  const state = cameras.get(frame.peerId);
  if (!state) return;
  const becameLive = state.status !== "live";
  state.streamStartedAtMs ??= Date.now();
  state.latestJpeg = frame.jpeg.slice();
  state.frameRevision += 1;
  state.received = frame.received;
  state.bytes = frame.bytes;
  state.timestampNs = frame.timestampNs;
  recordFrameDelivery(state, frame.jpeg.byteLength);
  state.status = "live";
  state.message = "";
  scheduleFrameDisplay(state);
  // Keep the receive loop mechanical. DOM/diagnostic work happens when the
  // browser finishes decoding a frame, not for frames the newest-only renderer
  // will intentionally skip.
  if (becameLive) updateCameraElement(state);
}

function showRemoteConnection(connection: RemoteConnection): void {
  const state = cameras.get(connection.target.peerId);
  if (!state) return;
  const previousQuality = state.connection?.metadata.profile.quality;
  if (previousQuality !== undefined && previousQuality !== connection.metadata.profile.quality) {
    resetStreamDiagnostics(state);
  }
  state.connection = connection;
  state.name = connection.metadata.info.name || state.name;
  state.status = "live";
  state.message = "Waiting for the first frame…";
  selectedPeerId = connection.target.peerId;
  renderWall();
  renderDiagnostics();
  record(
    `Info identified ${connection.metadata.info.name}; ${cameraQualityLabel(connection.metadata.profile.quality)} verified; available ${connection.metadata.availableQualities.join(", ")}`,
  );
}

function showSnapshot(snapshot: RemoteSnapshot): void {
  const state = cameras.get(snapshot.peerId);
  if (state) {
    state.snapshotPending = false;
    state.snapshotRequestId = undefined;
  }
  clearSnapshotUrl();
  snapshotObjectUrl = URL.createObjectURL(
    new Blob([snapshot.jpeg.slice().buffer], { type: "image/jpeg" }),
  );
  snapshotImage.src = snapshotObjectUrl;
  snapshotImage.hidden = false;
  snapshotTitle.textContent = state?.name ?? `Camera ${shortPeer(snapshot.peerId)}`;
  snapshotStatus.textContent = `${snapshot.jpeg.byteLength} bytes · ${snapshot.sha256} · ${snapshot.relayed ? "relay" : "direct"}`;
  renderWall();
  showModal(snapshotDialog);
}

function markCameraEnded(peerId: string, reason: string): void {
  const state = cameras.get(peerId);
  if (!state) return;
  state.status = "ended";
  state.sourcePaused = false;
  state.snapshotPending = false;
  state.message = reason;
  renderWall();
}

function updateCameraElement(state: CameraTileState): void {
  const tile = cameraElement(state.candidate.peerId);
  if (!tile) return;
  tile.dataset.status = state.status;
  tile.dataset.frameCount = String(state.received);
  if (state.connection) tile.dataset.quality = state.connection.metadata.profile.quality;
  setTileDiagnosticAttributes(tile, state);
  const surface = tile.querySelector<HTMLCanvasElement>("[data-role='remote-frame']");
  if (surface) {
    surface.hidden = !state.hasRenderedFrame;
    surface.classList.toggle("dimmed", state.status !== "live");
  }
  const time = tile.querySelector<HTMLElement>("[data-role='frame-time']");
  if (time && state.timestampNs) time.textContent = formatFrameTime(state.timestampNs);
  const metrics = tile.querySelector<HTMLElement>("[data-role='stream-diagnostics']");
  if (metrics) setTileDiagnostics(metrics, state);
  const status = tile.querySelector<HTMLElement>(".feed-status");
  if (status) status.textContent = statusLabel(state);
}

function renderPending(): void {
  if (pendingPeerIds.length === 0) {
    const empty = document.createElement("li");
    empty.className = "empty-row";
    empty.textContent = "No pending viewers.";
    pendingList.replaceChildren(empty);
    return;
  }
  const rows = pendingPeerIds.map((peerId) => {
    const row = document.createElement("li");
    const identity = document.createElement("code");
    identity.textContent = peerId;
    identity.title = peerId;
    const actions = document.createElement("div");
    actions.className = "button-row";
    const approve = document.createElement("button");
    approve.type = "button";
    approve.className = "primary";
    approve.textContent = "Allow";
    approve.addEventListener("click", () => mesh?.approve(peerId));
    const deny = document.createElement("button");
    deny.type = "button";
    deny.className = "secondary";
    deny.textContent = "Deny";
    deny.addEventListener("click", () => mesh?.deny(peerId));
    actions.append(approve, deny);
    row.append(identity, actions);
    return row;
  });
  pendingList.replaceChildren(...rows);
}

function selectCamera(peerId: string): void {
  if (!cameras.has(peerId) || selectedPeerId === peerId) return;
  selectedPeerId = peerId;
  for (const tile of cameraGrid.querySelectorAll<HTMLElement>(".camera-tile")) {
    tile.classList.toggle("selected", tile.dataset.cameraPeerId === peerId);
  }
  updateAggregateMetrics();
  renderDiagnostics();
}

function openDiagnostics(peerId?: string): void {
  if (peerId) selectedPeerId = peerId;
  selectedPeerId ??= cameraOrder[0];
  renderDiagnostics();
  showModal(diagnosticsDialog);
  document.querySelector<HTMLDetailsElement>(".session-menu")?.removeAttribute("open");
}

function renderDiagnostics(): void {
  const running = mesh;
  const state = selectedPeerId ? cameras.get(selectedPeerId) : undefined;
  updateLiveDiagnostics(state);
  if (state?.connection) {
    const connection = state.connection;
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
  } else if (state) {
    inspectorDetails.textContent = stringify({
      peerId: state.candidate.peerId,
      status: state.status,
      message: state.message,
    });
  } else if (running) {
    inspectorDetails.textContent = stringify({
      localPeerId: running.peerId,
      domainId: running.domainId,
      role: running.role,
      publishing: running.isPublishing,
      profiles: running.profiles,
    });
  } else {
    inspectorDetails.textContent = "Select a connected camera to inspect it.";
  }
}

function setColumnCount(value: number): void {
  if (!Number.isInteger(value) || value < 1 || value > 4 || value === columnCount) return;
  columnCount = value;
  selectedPeerId ??= cameraOrder[0];
  try {
    localStorage.setItem(COLUMN_STORAGE_KEY, String(value));
  } catch {
    // Column preference is optional.
  }
  renderWall();
}

function effectiveColumnCount(): number {
  return mobileLayout.matches && columnCount > 1 ? 2 : columnCount;
}

function changeFocusedCamera(offset: number): void {
  if (columnCount !== 1 || cameraOrder.length < 2) return;
  const current = selectedPeerId ? cameraOrder.indexOf(selectedPeerId) : 0;
  const normalized = current < 0 ? 0 : current;
  selectedPeerId = cameraOrder[
    (normalized + offset + cameraOrder.length) % cameraOrder.length
  ];
  renderWall();
}

function updateWallStatus(): void {
  const live = [...cameras.values()].filter((state) => state.status === "live").length;
  const pending = [...cameras.values()].filter(
    (state) => state.status === "connecting" || state.status === "awaiting",
  ).length;
  if (live > 0) {
    wallStatus.textContent = `${live} live${pending ? ` · ${pending} pending` : ""}`;
  } else if (pending > 0) {
    wallStatus.textContent = `${pending} pending`;
  } else {
    wallStatus.textContent = "No cameras";
  }
}

function updateAggregateMetrics(): void {
  const running = mesh;
  if (running?.role === "publisher") {
    const diagnostics = CAMERA_QUALITY_TIERS.flatMap((quality) => {
      const value = publisherDiagnostics.get(quality);
      return value ? [value] : [];
    });
    aggregateMetrics.textContent = diagnostics.length > 0
      ? `${diagnostics.map((value) => `${value.quality} ${formatRate(value.encodedFps)}`).join(" · ")} · ${formatBandwidth(diagnostics.reduce((total, value) => total + (value.kibPerSecond ?? 0), 0))}`
      : "Preparing low, medium, and high renditions";
    return;
  }
  const state = selectedPeerId ? cameras.get(selectedPeerId) : undefined;
  if (!state || state.received === 0) {
    aggregateMetrics.textContent = "Stream idle";
    return;
  }
  const diagnostics = streamDiagnostics(state);
  aggregateMetrics.textContent = `${state.received} received · receive ${formatRate(diagnostics.fps)} · display ${formatRate(diagnostics.displayFps)} · ${formatBandwidth(diagnostics.kibPerSecond)} · ${formatFrameSize(diagnostics.averageFrameKib)} · ${formatFrameAge(diagnostics.frameAgeMs)}`;
}

function resetStreamDiagnostics(state: CameraTileState): void {
  state.received = 0;
  state.bytes = 0;
  state.frameSamples = [];
  state.displaySamples = [];
  state.displayedFrameAgeMs = undefined;
  state.streamStartedAtMs = undefined;
}

function recordFrameDelivery(state: CameraTileState, bytes: number): void {
  const receivedAt = performance.now();
  state.frameSamples.push({ receivedAt, bytes });
  const cutoff = receivedAt - DIAGNOSTIC_WINDOW_MS;
  while (
    state.frameSamples.length > 2
    && state.frameSamples[1]!.receivedAt < cutoff
  ) {
    state.frameSamples.shift();
  }
  if (state.frameSamples.length > MAX_DIAGNOSTIC_SAMPLES) {
    state.frameSamples.splice(0, state.frameSamples.length - MAX_DIAGNOSTIC_SAMPLES);
  }
}

function streamDiagnostics(state: CameraTileState): {
  fps?: number;
  displayFps?: number;
  kibPerSecond?: number;
  averageFrameKib?: number;
  frameAgeMs?: number;
} {
  const samples = state.frameSamples;
  const displayFps = sampleRate(state.displaySamples);
  if (samples.length === 0) return { displayFps, frameAgeMs: state.displayedFrameAgeMs };
  const totalBytes = samples.reduce((total, sample) => total + sample.bytes, 0);
  const averageFrameKib = totalBytes / samples.length / 1_024;
  if (samples.length === 1) {
    return { displayFps, averageFrameKib, frameAgeMs: state.displayedFrameAgeMs };
  }
  const first = samples[0]!;
  const last = samples[samples.length - 1]!;
  const elapsedSeconds = Math.max(0.001, (last.receivedAt - first.receivedAt) / 1_000);
  const deliveredBytes = totalBytes - first.bytes;
  return {
    fps: (samples.length - 1) / elapsedSeconds,
    displayFps,
    kibPerSecond: deliveredBytes / elapsedSeconds / 1_024,
    averageFrameKib,
    frameAgeMs: state.displayedFrameAgeMs,
  };
}

function setTileDiagnostics(element: HTMLElement, state: CameraTileState): void {
  if (state.received === 0) {
    element.textContent = "Waiting for frames";
    element.title = "Rolling five-second stream diagnostics";
    return;
  }
  const diagnostics = streamDiagnostics(state);
  element.textContent = [
    `receive ${formatRate(diagnostics.fps)}`,
    `display ${formatRate(diagnostics.displayFps)}`,
    formatBandwidth(diagnostics.kibPerSecond),
    formatFrameAge(diagnostics.frameAgeMs),
  ].join(" · ");
  element.title = `${formatFrameSize(diagnostics.averageFrameKib)} · rolling five-second receive window`;
}

function setTileDiagnosticAttributes(tile: HTMLElement, state: CameraTileState): void {
  const diagnostics = streamDiagnostics(state);
  setFiniteDataset(tile, "streamFps", diagnostics.fps);
  setFiniteDataset(tile, "displayFps", diagnostics.displayFps);
  setFiniteDataset(tile, "kibPerSecond", diagnostics.kibPerSecond);
  setFiniteDataset(tile, "averageFrameKib", diagnostics.averageFrameKib);
  setFiniteDataset(tile, "frameAgeMs", diagnostics.frameAgeMs);
}

function setFiniteDataset(
  element: HTMLElement,
  name: "streamFps" | "displayFps" | "kibPerSecond" | "averageFrameKib" | "frameAgeMs",
  value: number | undefined,
): void {
  if (value === undefined || !Number.isFinite(value)) {
    delete element.dataset[name];
  } else {
    element.dataset[name] = value.toFixed(2);
  }
}

function updateLiveDiagnostics(state: CameraTileState | undefined): void {
  const running = mesh;
  const publisherValues = CAMERA_QUALITY_TIERS.flatMap((quality) => {
    const value = publisherDiagnostics.get(quality);
    return value ? [value] : [];
  });
  const highDiagnostics = publisherDiagnostics.get("high") ?? publisherValues.at(-1);
  diagnosticProfile.textContent = state?.connection
    ? cameraQualityLabel(state.connection.metadata.profile.quality)
    : running?.role === "publisher"
      ? "Low, medium, and high"
      : "—";
  diagnosticCaptureFps.textContent = publisherValues.length > 0
    ? publisherValues.map((value) => `${value.quality[0]?.toUpperCase()} ${formatRate(value.encodedFps)}`).join(" · ")
    : "—";
  diagnosticCaptureDetail.textContent = highDiagnostics
    ? `high target ${highDiagnostics.targetFps}${highDiagnostics.inputFps ? ` · input ${highDiagnostics.inputFps}` : ""} · encode p50/p95 ${formatDuration(highDiagnostics.encodeP50Ms)} / ${formatDuration(highDiagnostics.encodeP95Ms)} · ${highDiagnostics.missedDeadlines} missed`
    : "publisher only";
  if (!state || state.received === 0) {
    diagnosticFps.textContent = "—";
    diagnosticDisplayFps.textContent = "—";
    diagnosticBandwidth.textContent = publisherValues.length > 0
      ? formatBandwidth(publisherValues.reduce(
        (total, value) => total + (value.kibPerSecond ?? 0),
        0,
      ))
      : "—";
    diagnosticFrameSize.textContent = highDiagnostics
      ? `high ${formatFrameSize(highDiagnostics.averageFrameKib)}`
      : "Waiting for frames";
    diagnosticFrameAge.textContent = "—";
    return;
  }
  const diagnostics = streamDiagnostics(state);
  diagnosticFps.textContent = formatRate(diagnostics.fps);
  diagnosticDisplayFps.textContent = formatRate(diagnostics.displayFps);
  diagnosticBandwidth.textContent = formatBandwidth(diagnostics.kibPerSecond);
  diagnosticFrameSize.textContent = formatFrameSize(diagnostics.averageFrameKib);
  diagnosticFrameAge.textContent = formatFrameAge(diagnostics.frameAgeMs);
}

function scheduleFrameDisplay(state: CameraTileState): void {
  if (
    state.renderingFrame
    || state.frozen
    || !state.latestJpeg
    || state.timestampNs === undefined
    || (state.hasRenderedFrame && state.renderedRevision >= state.frameRevision)
    || !cameraElement(state.candidate.peerId)
  ) return;
  state.renderingFrame = true;
  void decodeAndDisplayLatest(state);
}

async function decodeAndDisplayLatest(state: CameraTileState): Promise<void> {
  const revision = state.frameRevision;
  const generation = state.displayGeneration;
  const jpeg = state.latestJpeg;
  const timestampNs = state.timestampNs;
  if (!jpeg || timestampNs === undefined) {
    state.renderingFrame = false;
    return;
  }

  const blob = new Blob([jpeg.slice().buffer], { type: "image/jpeg" });
  let bitmap: ImageBitmap | undefined;
  let frameUrl: string | undefined;
  let adoptedUrl = false;
  try {
    bitmap = await createImageBitmap(blob);
    const tile = cameraElement(state.candidate.peerId);
    if (
      cameras.get(state.candidate.peerId) !== state
      || state.displayGeneration !== generation
      || state.frozen
      || !tile
    ) return;

    const surface = state.frameSurface;
    if (!tile.contains(surface)) return;
    if (surface.width !== bitmap.width || surface.height !== bitmap.height) {
      surface.width = bitmap.width;
      surface.height = bitmap.height;
    }
    const context = surface.getContext("2d", { alpha: false });
    if (!context) throw new Error("browser could not create the camera canvas");
    context.drawImage(bitmap, 0, 0, surface.width, surface.height);

    // Keep the compressed frame available for interoperability smoke hashing.
    // The visible feed is the persistent canvas and never consumes this URL.
    frameUrl = URL.createObjectURL(blob);
    const previous = state.frameUrl;
    state.frameUrl = frameUrl;
    surface.dataset.frameUrl = frameUrl;
    surface.dataset.renderedRevision = String(revision);
    state.hasRenderedFrame = true;
    state.renderedRevision = revision;
    state.displayedFrameAgeMs = Date.now() - Number(timestampNs / 1_000_000n);
    adoptedUrl = true;
    tile.dataset.status = state.status;
    tile.dataset.frameCount = String(state.received);
    surface.hidden = false;
    surface.classList.toggle("dimmed", state.status !== "live");
    // A frame decode can finish after the stream has ended. Keep the offline
    // message and Retry action rendered by markCameraEnded in that race.
    if (state.status === "live") {
      tile.querySelector<HTMLElement>(".tile-center")?.replaceChildren();
    }
    recordFrameDisplay(state);

    const time = tile.querySelector<HTMLElement>("[data-role='frame-time']");
    if (time) time.textContent = formatFrameTime(timestampNs);
    const metrics = tile.querySelector<HTMLElement>("[data-role='stream-diagnostics']");
    if (metrics) setTileDiagnostics(metrics, state);
    const status = tile.querySelector<HTMLElement>(".feed-status");
    if (status) status.textContent = statusLabel(state);
    setTileDiagnosticAttributes(tile, state);
    if (selectedPeerId === state.candidate.peerId) {
      updateAggregateMetrics();
      updateLiveDiagnostics(state);
    }
    if (previous) revokeAfterPaint(previous);
  } catch (error) {
    state.renderedRevision = Math.max(state.renderedRevision, revision);
    record(`Frame decode failed for ${shortPeer(state.candidate.peerId)}: ${errorMessage(error)}`);
  } finally {
    bitmap?.close();
    if (frameUrl && !adoptedUrl) URL.revokeObjectURL(frameUrl);
    state.renderingFrame = false;
    if (state.renderedRevision < state.frameRevision) scheduleFrameDisplay(state);
  }
}

function revokeAfterPaint(frameUrl: string): void {
  requestAnimationFrame(() => requestAnimationFrame(() => URL.revokeObjectURL(frameUrl)));
}

function recordFrameDisplay(state: CameraTileState): void {
  const displayedAt = performance.now();
  state.displaySamples.push(displayedAt);
  const cutoff = displayedAt - DIAGNOSTIC_WINDOW_MS;
  while (state.displaySamples.length > 2 && state.displaySamples[1]! < cutoff) {
    state.displaySamples.shift();
  }
  if (state.displaySamples.length > MAX_DIAGNOSTIC_SAMPLES) {
    state.displaySamples.splice(0, state.displaySamples.length - MAX_DIAGNOSTIC_SAMPLES);
  }
}

function sampleRate(samples: readonly number[]): number | undefined {
  if (samples.length < 2) return undefined;
  const first = samples[0]!;
  const last = samples[samples.length - 1]!;
  return (samples.length - 1) / Math.max(0.001, (last - first) / 1_000);
}

function clearCameraSurface(state: CameraTileState): void {
  state.displayGeneration += 1;
  if (state.frameUrl) URL.revokeObjectURL(state.frameUrl);
  state.frameUrl = undefined;
  state.hasRenderedFrame = false;
  delete state.frameSurface.dataset.frameUrl;
  delete state.frameSurface.dataset.renderedRevision;
  state.frameSurface.hidden = true;
  state.frameSurface.width = 1;
  state.frameSurface.height = 1;
}

function clearAllCameraSurfaces(): void {
  for (const state of cameras.values()) clearCameraSurface(state);
}

function clearSnapshotUrl(): void {
  if (snapshotObjectUrl) URL.revokeObjectURL(snapshotObjectUrl);
  snapshotObjectUrl = undefined;
  snapshotImage.removeAttribute("src");
  snapshotImage.hidden = true;
}

function cameraElement(peerId: string): HTMLElement | null {
  return cameraGrid.querySelector<HTMLElement>(
    `[data-camera-peer-id="${CSS.escape(peerId)}"]`,
  );
}

async function stopPeer(): Promise<void> {
  const running = mesh;
  if (!running) return;
  mesh = undefined;
  publisherDiagnostics.clear();
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
    clearAllCameraSurfaces();
    clearSnapshotUrl();
    localCard.textContent = stopped ? "Peer stopped" : "Peer shutdown failed — see events";
    peerStatusDot.className = stopped ? "status-dot" : "status-dot error";
    wallStatus.textContent = stopped ? "Peer stopped" : "Shutdown failed";
    viewerToolbar.hidden = true;
    publisherPanel.hidden = true;
    viewerPanel.hidden = true;
    for (const candidate of [addDialog, diagnosticsDialog, snapshotDialog, cameraActionsDialog]) {
      if (candidate.open) candidate.close();
    }
  }
}

function renderCard(): void {
  localCard.textContent = mesh ? JSON.stringify(mesh.card(), null, 2) : "No peer running";
}

async function copyCard(): Promise<void> {
  const card: CameraPeerCard | undefined = mesh?.card();
  if (!card) return;
  try {
    await navigator.clipboard.writeText(JSON.stringify(card, null, 2));
    record("Sanitized peer card copied");
    showToast("Peer card copied");
  } catch (error) {
    report(error);
  }
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
  if (card["domainId"] !== running.domainId) throw new Error("Peer card belongs to another Domain");
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

function record(message: string): void {
  const stamp = new Date().toLocaleTimeString();
  timelineRows.push(`${stamp}  ${message}`);
  if (timelineRows.length > 80) timelineRows.splice(0, timelineRows.length - 80);
  timeline.textContent = timelineRows.join("\n");
  eventRows.push(message);
  if (eventRows.length > 24) eventRows.splice(0, eventRows.length - 24);
  events.textContent = eventRows.slice().reverse().join("\n");
}

function report(error: unknown, notify = true): void {
  const message = errorMessage(error);
  record(`ERROR · ${message}`);
  if (notify) showToast(message, true);
}

function showToast(message: string, error = false): void {
  if (toastTimer !== undefined) window.clearTimeout(toastTimer);
  toast.textContent = message;
  toast.classList.toggle("error", error);
  toast.hidden = false;
  toastTimer = window.setTimeout(() => {
    toast.hidden = true;
    toastTimer = undefined;
  }, 5_000);
}

function showInlineError(element: HTMLElement, message: string): void {
  element.textContent = message;
  element.hidden = false;
}

function clearInlineError(element: HTMLElement): void {
  element.textContent = "";
  element.hidden = true;
}

function showModal(target: HTMLDialogElement): void {
  if (!target.open) target.showModal();
}

function initialColumnCount(): number {
  try {
    const stored = Number(localStorage.getItem(COLUMN_STORAGE_KEY));
    if (Number.isInteger(stored) && stored >= 1 && stored <= 4) return stored;
  } catch {
    // Column preference is optional.
  }
  return 2;
}

function statusLabel(state: CameraTileState): string {
  if (state.frozen) return "Frozen";
  if (state.sourcePaused) return "Source paused";
  if (state.switchingQuality) return `Switching → ${state.switchingQuality}`;
  if (state.status === "awaiting") return "Approval";
  if (state.status === "connecting") return "Connecting";
  if (state.status === "ended") return "Offline";
  if (state.status === "error") return "Error";
  const quality = state.connection?.metadata.profile.quality;
  const live = quality ? `Live · ${quality}` : "Live";
  return state.streamStartedAtMs === undefined
    ? live
    : `${live} · ${formatStreamAge(Date.now() - state.streamStartedAtMs)}`;
}

function centerHeading(state: CameraTileState): string {
  if (state.status === "awaiting") return "Waiting for approval";
  if (state.status === "connecting") return "Connecting";
  if (state.status === "ended") return "Camera offline";
  if (state.status === "error") return "Could not connect";
  return "Waiting for video";
}

function readableConnectionError(message: string): string {
  if (message.includes("access_denied")) return "The publisher denied camera access.";
  if (message.includes("no browser-compatible WSS route")) return "This camera has no Web route.";
  return "The camera is unavailable. Check its route and try again.";
}

function selectedDomainName(): string {
  const label = domain.selectedOptions[0]?.textContent?.trim();
  return label?.split(" — ")[0] || domain.value;
}

function formatFrameTime(timestampNs: bigint): string {
  const milliseconds = Number(timestampNs / 1_000_000n);
  return new Date(milliseconds).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

function formatRate(value: number | undefined): string {
  return value === undefined ? "FPS —" : `${value.toFixed(1)} fps`;
}

function formatBandwidth(value: number | undefined): string {
  return value === undefined ? "Bandwidth —" : `${value.toFixed(1)} KiB/s`;
}

function formatFrameSize(value: number | undefined): string {
  return value === undefined ? "Frame size —" : `${value.toFixed(1)} KiB/frame`;
}

function formatFrameAge(value: number | undefined): string {
  if (value === undefined) return "Age —";
  const rounded = Math.round(value);
  return `${rounded >= 0 ? rounded : `−${Math.abs(rounded)}`} ms age`;
}

function formatDuration(value: number | undefined): string {
  return value === undefined ? "—" : `${value.toFixed(1)} ms`;
}

function formatStreamAge(milliseconds: number): string {
  const seconds = Math.max(0, Math.floor(milliseconds / 1_000));
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  const remainder = seconds % 60;
  if (minutes < 60) return `${minutes}m ${String(remainder).padStart(2, "0")}s`;
  const hours = Math.floor(minutes / 60);
  return `${hours}h ${String(minutes % 60).padStart(2, "0")}m`;
}

function formatExpiry(value: string): string {
  const parsed = Date.parse(value);
  if (!Number.isFinite(parsed)) return value;
  const seconds = Math.max(0, Math.round((parsed - Date.now()) / 1_000));
  return seconds < 60 ? `${seconds}s` : `${Math.round(seconds / 60)}m`;
}

function requiredString(value: unknown, label: string): string {
  if (typeof value !== "string" || value.length === 0) throw new Error(`${label} is missing`);
  return value;
}

function defaultName(): string {
  return role.value === "publisher" ? "Browser camera" : "Camera wall";
}

function preferredQualityForWall(): CameraQualityTier {
  const columns = effectiveColumnCount();
  return columns === 1 ? "high" : columns === 2 ? "medium" : "low";
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
