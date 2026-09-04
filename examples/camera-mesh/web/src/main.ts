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
  LatestJpegRenderer,
  type CameraFrameSurface,
  type JpegPresentation,
} from "./jpeg-renderer.js";
import {
  CAMERA_QUALITY_TIERS,
  cameraAddAllLimit,
  cameraQualityLabel,
  isCameraQualityTier,
  type CameraQualityTier,
} from "./profile.js";
import {
  CAMERA_PERFORMANCE_SAMPLE_INTERVAL_MS,
  CameraPerformanceCapture,
  cameraPerformanceReportFilename,
  serializeCameraPerformanceReport,
  type CameraPerformanceReport,
  type CameraPerformanceSnapshot,
} from "./performance-report.js";

type CameraStatus = "connecting" | "awaiting" | "live" | "ended" | "error";

interface FrameDeliverySample {
  readonly receivedAt: number;
  readonly bytes: number;
  readonly sourceTimestampNs: bigint;
}

interface EventLoopDelaySample {
  readonly observedAt: number;
  readonly delayMs: number;
}

interface CameraTileState {
  readonly candidate: CameraCandidate;
  preferredQuality: CameraQualityTier;
  status: CameraStatus;
  message: string;
  name: string;
  operation: number;
  connection?: RemoteConnection;
  readonly frameSurface: CameraFrameSurface;
  readonly frameRenderer: LatestJpegRenderer;
  hasRenderedFrame: boolean;
  frameRevision: number;
  renderVisible: boolean;
  received: number;
  bytes: number;
  totalReceivedFrames: number;
  totalRenderedFrames: number;
  totalReceivedBytes: number;
  frameSamples: FrameDeliverySample[];
  displaySamples: number[];
  displayedTimestampNs?: bigint;
  streamStartedAtMs?: number;
  timestampNs?: bigint;
  frozen: boolean;
  sourcePaused: boolean;
  snapshotPending: boolean;
  snapshotRequestId?: string;
  switchingQuality?: CameraQualityTier;
}

interface StreamDiagnostics {
  fps?: number;
  displayFps?: number;
  kibPerSecond?: number;
  averageFrameKib?: number;
  frameAgeMs?: number;
  sourceGapP95Ms?: number;
  sourceGapMaxMs?: number;
  receiveGapP95Ms?: number;
  receiveGapMaxMs?: number;
  renderGapP95Ms?: number;
  renderGapMaxMs?: number;
  queueMs?: number;
  queueP95Ms?: number;
  queueMaxMs?: number;
  decodeMs?: number;
  decodeP50Ms?: number;
  decodeP95Ms?: number;
  decodeMaxMs?: number;
  presentMs?: number;
  displayWidth?: number;
  displayHeight?: number;
  renderer?: string;
  rendererEnabled: boolean;
  decodeInFlight: boolean;
  pendingFrames: number;
  activeDecodes: number;
  queuedRenderers: number;
  maximumActiveDecodes: number;
  supersededFrames: number;
  queueOverflowFrames: number;
}

interface GapStatistics {
  readonly p95: number;
  readonly maximum: number;
}

const MAX_CAMERAS = 16;
const DIAGNOSTIC_WINDOW_MS = 5_000;
const MAX_DIAGNOSTIC_SAMPLES = 512;
const EVENT_LOOP_PROBE_INTERVAL_MS = 50;
const LIVE_TELEMETRY_INTERVAL_MS = 250;
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
const viewerInboundRelay = input("viewer-inbound-relay");
const viewerInboundRelayField = get<HTMLElement>("viewer-inbound-relay-field");
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
const performanceReportDialog = dialog("performance-report-dialog");
const snapshotDialog = dialog("snapshot-dialog");
const cameraActionsDialog = dialog("camera-actions-dialog");
const cameraActionsTitle = get<HTMLElement>("camera-actions-title");
const cameraResults = get<HTMLElement>("camera-results");
const addAllCamerasButton = button("add-all-cameras-button");
const removeAllCamerasButton = button("remove-all-cameras-button");
const addCameraQuality = get<HTMLSelectElement>("add-camera-quality");
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
const recordPerformanceButton = button("record-performance-button");
const recordPerformanceLabel = get<HTMLElement>("record-performance-label");
const openPerformanceReportButton = button("open-performance-report-button");
const performanceReportSummary = get<HTMLElement>("performance-report-summary");

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
let removingAllCameras = false;
let performanceCapture: CameraPerformanceCapture | undefined;
let performanceCaptureTimer: number | undefined;
let eventLoopProbeTimer: number | undefined;
let eventLoopDelaySamples: EventLoopDelaySample[] = [];
let liveTelemetryTimer: number | undefined;
let completedPerformanceReport: CameraPerformanceReport | undefined;
const publisherDiagnostics = new Map<CameraQualityTier, CaptureDiagnostics>();
const timelineRows: string[] = [];
const eventRows: string[] = [];
const cameraVisibilityObserver = typeof IntersectionObserver === "undefined"
  ? undefined
  : new IntersectionObserver(handleCameraVisibility);

await init();
loginButton.disabled = false;
record("Rust/Wasm runtime ready");
renderWall();

loginForm.addEventListener("submit", (event) => {
  event.preventDefault();
  void login();
});
role.addEventListener("change", renderReachabilityOption);
button("start-peer-button").addEventListener("click", () => void startPeer());
button("copy-card-button").addEventListener("click", () => void copyCard());
button("open-diagnostics-button").addEventListener("click", () => openDiagnostics());
button("stop-peer-button").addEventListener("click", () => void stopPeer());
button("publish-button").addEventListener("click", () => void publish());
button("stop-publish-button").addEventListener("click", () => void stopPublishing());
button("add-camera-button").addEventListener("click", openAddCamera);
button("discover-button").addEventListener("click", () => void discover());
addAllCamerasButton.addEventListener("click", () => void addAllCameras());
addCameraQuality.addEventListener("change", renderCandidates);
removeAllCamerasButton.addEventListener("click", () => void removeAllCameras());
recordPerformanceButton.addEventListener("click", togglePerformanceRecording);
openPerformanceReportButton.addEventListener("click", openPerformanceReport);
button("close-performance-report-button").addEventListener("click", () => performanceReportDialog.close());
button("copy-performance-report-button").addEventListener("click", () => void copyPerformanceReport());
button("download-performance-report-button").addEventListener("click", downloadPerformanceReport);
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
document.addEventListener("visibilitychange", () => {
  performanceCapture?.recordEvent(`Page visibility changed to ${document.visibilityState}`);
  refreshCameraVisibility();
});
renderReachabilityOption();

cameraResults.addEventListener("click", (event) => {
  const control = (event.target as Element).closest<HTMLButtonElement>("button[data-candidate-peer-id]");
  if (!control) return;
  const candidate = candidates.get(control.dataset.candidatePeerId ?? "");
  if (candidate) void addCamera(candidate, selectedAddQuality());
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
      { viewerInboundRelay: viewerInboundRelay.checked },
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
    record(
      `${capitalized(started.role)} peer ${shortPeer(started.peerId)} is ready (${started.relayBacked ? "relay-backed" : "outbound-only"})`,
    );
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

function renderReachabilityOption(): void {
  viewerInboundRelayField.hidden = role.value !== "viewer";
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
  addCameraQuality.value = preferredQualityForWall();
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
    await addCamera(candidate, selectedAddQuality());
  } catch (error) {
    showInlineError(addCameraError, errorMessage(error));
    report(error, false);
  }
}

async function addAllCameras(): Promise<void> {
  if (addingAllCameras) return;
  const preferredQuality = selectedAddQuality();
  const available = addableCandidates();
  const limit = cameraAddAllLimit(preferredQuality);
  const additions = addAllCandidates(preferredQuality, available);
  if (additions.length === 0) return;

  addingAllCameras = true;
  addAllCamerasButton.disabled = true;
  addAllCamerasButton.textContent = `Adding ${additions.length}…`;
  record(
    `Connecting ${additions.length} of ${available.length} discovered camera(s) at ${cameraQualityLabel(preferredQuality)} (batch target ${limit})`,
  );
  addDialog.close();
  try {
    const results = await Promise.allSettled(
      additions.map((candidate) => addCamera(candidate, preferredQuality)),
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

function addAllCandidates(
  quality: CameraQualityTier,
  available = addableCandidates(),
): CameraCandidate[] {
  const active = [...cameras.values()].filter(
    (state) => state.status === "connecting" || state.status === "live",
  ).length;
  const remainingTargetSlots = Math.max(0, cameraAddAllLimit(quality) - active);
  return available.slice(0, remainingTargetSlots);
}

async function addCamera(
  candidate: CameraCandidate,
  requestedQuality?: CameraQualityTier,
): Promise<void> {
  const running = mesh;
  if (!running || running.role !== "viewer") return;
  let state = cameras.get(candidate.peerId);
  const preferredQuality = requestedQuality ?? state?.preferredQuality ?? preferredQualityForWall();
  const existing = state !== undefined;
  if (!state) {
    if (cameras.size >= MAX_CAMERAS) {
      showInlineError(addCameraError, "The camera wall already contains 16 cameras");
      return;
    }
    state = createCameraTileState(candidate, preferredQuality);
    cameras.set(candidate.peerId, state);
    cameraOrder.push(candidate.peerId);
    samplePerformance([state]);
  }
  if (existing && (state.status === "live" || state.status === "connecting")) {
    selectedPeerId = candidate.peerId;
    renderWall();
    addDialog.close();
    return;
  }

  const operation = ++state.operation;
  state.preferredQuality = preferredQuality;
  state.status = "connecting";
  state.message = "Verifying camera metadata and opening the Stream…";
  resetStreamDiagnostics(state);
  state.sourcePaused = false;
  selectedPeerId = candidate.peerId;
  renderWall();
  addDialog.close();
  try {
    const connection = await running.connectCamera(candidate, preferredQuality);
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

function createCameraTileState(
  candidate: CameraCandidate,
  preferredQuality: CameraQualityTier,
): CameraTileState {
  const frameSurface = document.createElement("canvas") as CameraFrameSurface;
  let state!: CameraTileState;
  const frameRenderer = new LatestJpegRenderer(frameSurface, {
    presented: (presentation) => showPresentedFrame(state, presentation),
    failed: (error) => {
      record(`Frame decode failed for ${shortPeer(candidate.peerId)}: ${errorMessage(error)}`);
    },
  });
  state = {
    candidate,
    preferredQuality,
    status: "connecting",
    message: "Verifying camera metadata and opening the Stream…",
    name: `Camera ${shortPeer(candidate.peerId)}`,
    operation: 0,
    received: 0,
    bytes: 0,
    totalReceivedFrames: 0,
    totalRenderedFrames: 0,
    totalReceivedBytes: 0,
    frameSamples: [],
    displaySamples: [],
    displayedTimestampNs: undefined,
    frameSurface,
    frameRenderer,
    hasRenderedFrame: false,
    frameRevision: 0,
    renderVisible: false,
    frozen: false,
    sourcePaused: false,
    snapshotPending: false,
  };
  return state;
}

async function handleTileAction(action: string, peerId: string): Promise<void> {
  const state = cameras.get(peerId);
  if (!state) return;
  if (action === "menu") {
    openCameraActions(peerId);
  } else if (action === "retry") {
    await addCamera(state.candidate, state.preferredQuality);
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
    state.preferredQuality = quality;
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
  samplePerformance([state]);
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

async function removeAllCameras(): Promise<void> {
  const running = mesh;
  if (
    !running
    || running.role !== "viewer"
    || removingAllCameras
    || cameras.size === 0
  ) return;

  removingAllCameras = true;
  const removed = [...cameras.values()];
  samplePerformance(removed);
  for (const state of removed) {
    state.operation += 1;
    clearCameraSurface(state);
  }
  cameras.clear();
  cameraOrder = [];
  selectedPeerId = undefined;
  actionPeerId = undefined;
  if (cameraActionsDialog.open) cameraActionsDialog.close();
  document.querySelector<HTMLDetailsElement>(".session-menu")?.removeAttribute("open");
  renderWall();
  renderCandidates();

  try {
    const results = await Promise.allSettled(
      removed.map((state) => running.disconnectCamera(state.candidate.peerId)),
    );
    const failures = results.filter((result) => result.status === "rejected");
    for (const failure of failures) {
      if (failure.status === "rejected") report(failure.reason, false);
    }
    if (failures.length > 0) {
      record(
        `Removed ${removed.length} camera(s); ${failures.length} Stream disconnect(s) reported errors`,
      );
      showToast(
        `Removed ${removed.length} camera${removed.length === 1 ? "" : "s"} · ${failures.length} disconnect failed`,
        true,
      );
    } else {
      record(`Disconnected and removed ${removed.length} camera(s)`);
      showToast(`Removed ${removed.length} camera${removed.length === 1 ? "" : "s"}`);
    }
  } finally {
    removingAllCameras = false;
    renderWall();
    renderCandidates();
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
  if (!running.supportsSnapshots) {
    showToast("Restart Monitor mode with an inbound relay to request snapshots", true);
    return;
  }
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
  const quality = selectedAddQuality();
  const limit = cameraAddAllLimit(quality);
  const addable = addAllCandidates(quality);
  addAllCamerasButton.disabled = addingAllCameras || removingAllCameras || addable.length === 0;
  addAllCamerasButton.textContent = addingAllCameras
    ? "Adding…"
    : addable.length > 0
      ? `Add all (${addable.length})`
      : "Add all";
  addAllCamerasButton.title = `Connect to up to ${limit} discovered camera${limit === 1 ? "" : "s"} at ${cameraQualityLabel(quality)}`;
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
  cameraVisibilityObserver?.disconnect();
  if (selectedPeerId && !cameras.has(selectedPeerId)) selectedPeerId = undefined;
  selectedPeerId ??= cameraOrder[0];
  const renderedColumns = effectiveColumnCount();
  const focusMode = renderedColumns === 1;
  const visiblePeerIds = focusMode
    ? selectedPeerId ? [selectedPeerId] : []
    : [...cameraOrder];
  const visible = new Set(visiblePeerIds);
  for (const [peerId, state] of cameras) {
    if (!visible.has(peerId)) {
      state.renderVisible = false;
      clearCameraSurface(state);
    }
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
  refreshCameraVisibility();

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
  button("add-camera-button").disabled = removingAllCameras || cameras.size >= MAX_CAMERAS;
  removeAllCamerasButton.hidden = mesh?.role !== "viewer";
  removeAllCamerasButton.disabled = removingAllCameras || cameras.size === 0;
  removeAllCamerasButton.textContent = removingAllCameras
    ? "Disconnecting cameras…"
    : cameras.size > 0
      ? `Disconnect and remove all cameras (${cameras.size})`
      : "Disconnect and remove all cameras";
  updateWallStatus();
  updateAggregateMetrics();
}

function handleCameraVisibility(entries: readonly IntersectionObserverEntry[]): void {
  for (const entry of entries) {
    const tile = entry.target as HTMLElement;
    const peerId = tile.dataset.cameraPeerId;
    if (!peerId) continue;
    const state = cameras.get(peerId);
    if (!state || cameraElement(peerId) !== tile) continue;
    state.renderVisible = entry.isIntersecting && entry.intersectionRatio > 0;
    state.frameRenderer.setEnabled(
      document.visibilityState === "visible" && state.renderVisible && !state.frozen,
    );
  }
}

function refreshCameraVisibility(): void {
  cameraVisibilityObserver?.disconnect();
  for (const peerId of cameraOrder) {
    const state = cameras.get(peerId);
    const tile = cameraElement(peerId);
    if (!state || !tile) continue;
    const bounds = tile.getBoundingClientRect();
    state.renderVisible = document.visibilityState === "visible"
      && bounds.width > 0
      && bounds.height > 0
      && bounds.bottom > 0
      && bounds.right > 0
      && bounds.top < window.innerHeight
      && bounds.left < window.innerWidth;
    state.frameRenderer.setEnabled(state.renderVisible && !state.frozen);
    cameraVisibilityObserver?.observe(tile);
  }
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
      tileAction(
        "snapshot",
        peerId,
        "◎",
        mesh?.supportsSnapshots ? "Verified snapshot" : "Verified snapshots require an inbound relay",
        state.snapshotPending || !mesh?.supportsSnapshots,
      ),
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
      control.disabled = state.snapshotPending || !mesh?.supportsSnapshots;
      control.title = mesh?.supportsSnapshots
        ? "Request a verified snapshot"
        : "Restart Monitor mode with an inbound relay to request snapshots";
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
  state.frameRevision += 1;
  state.received = frame.received;
  state.bytes = frame.bytes;
  state.totalReceivedFrames += 1;
  state.totalReceivedBytes += frame.jpeg.byteLength;
  state.timestampNs = frame.timestampNs;
  recordFrameDelivery(state, frame.jpeg.byteLength, frame.timestampNs);
  state.status = "live";
  state.message = "";
  state.frameRenderer.submit({
    jpeg: frame.jpeg,
    revision: state.frameRevision,
    timestampNs: frame.timestampNs,
    sourceWidth: frame.profile.width,
    sourceHeight: frame.profile.height,
    rateHz: frame.profile.rateHz,
  });
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
    state.frameRenderer.invalidate();
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
        browserRenderer: state.frameRenderer.metrics(),
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
  state.displayedTimestampNs = undefined;
  state.streamStartedAtMs = undefined;
  state.frameRenderer.resetMeasurements();
}

function recordFrameDelivery(
  state: CameraTileState,
  bytes: number,
  sourceTimestampNs: bigint,
): void {
  const receivedAt = performance.now();
  state.frameSamples.push({ receivedAt, bytes, sourceTimestampNs });
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

function streamDiagnostics(state: CameraTileState, detailed = false): StreamDiagnostics {
  const now = performance.now();
  const renderer = state.frameRenderer.metrics(now, detailed);
  const samples = windowedFrameSamples(state.frameSamples, now);
  const displaySamples = windowedTimestamps(state.displaySamples, now);
  const displayFps = sampleRate(displaySamples, now);
  const sourceGaps = detailed ? sourceGapStatistics(samples) : undefined;
  const receiveGaps = detailed
    ? gapStatistics(samples.map((sample) => sample.receivedAt), now)
    : undefined;
  const renderGaps = detailed ? gapStatistics(displaySamples, now) : undefined;
  const renderDiagnostics = {
    sourceGapP95Ms: sourceGaps?.p95,
    sourceGapMaxMs: sourceGaps?.maximum,
    receiveGapP95Ms: receiveGaps?.p95,
    receiveGapMaxMs: receiveGaps?.maximum,
    renderGapP95Ms: renderGaps?.p95,
    renderGapMaxMs: renderGaps?.maximum,
    queueMs: renderer.queueMs,
    queueP95Ms: renderer.queueP95Ms,
    queueMaxMs: renderer.queueMaxMs,
    decodeMs: renderer.decodeMs,
    decodeP50Ms: renderer.decodeP50Ms,
    decodeP95Ms: renderer.decodeP95Ms,
    decodeMaxMs: renderer.decodeMaxMs,
    presentMs: renderer.presentMs,
    displayWidth: renderer.displayWidth,
    displayHeight: renderer.displayHeight,
    renderer: renderer.backend,
    rendererEnabled: renderer.enabled,
    decodeInFlight: renderer.decodeInFlight,
    pendingFrames: renderer.pendingFrames,
    activeDecodes: renderer.activeDecodes,
    queuedRenderers: renderer.queuedRenderers,
    maximumActiveDecodes: renderer.maximumActiveDecodes,
    supersededFrames: renderer.totalSupersededFrames,
    queueOverflowFrames: renderer.totalQueueOverflowFrames,
  };
  const frameAgeMs = displayedFrameAge(state);
  if (samples.length === 0) {
    return {
      displayFps,
      frameAgeMs,
      ...renderDiagnostics,
    };
  }
  const totalBytes = samples.reduce((total, sample) => total + sample.bytes, 0);
  const averageFrameKib = totalBytes / samples.length / 1_024;
  if (samples.length === 1) {
    return {
      displayFps,
      averageFrameKib,
      frameAgeMs,
      ...renderDiagnostics,
    };
  }
  const first = samples[0]!;
  const elapsedSeconds = Math.max(0.001, (now - first.receivedAt) / 1_000);
  const deliveredBytes = totalBytes - first.bytes;
  return {
    fps: (samples.length - 1) / elapsedSeconds,
    displayFps,
    kibPerSecond: deliveredBytes / elapsedSeconds / 1_024,
    averageFrameKib,
    frameAgeMs,
    ...renderDiagnostics,
  };
}

function displayedFrameAge(state: CameraTileState): number | undefined {
  return state.displayedTimestampNs === undefined
    ? undefined
    : Date.now() - Number(state.displayedTimestampNs / 1_000_000n);
}

function windowedFrameSamples(
  samples: readonly FrameDeliverySample[],
  now: number,
): readonly FrameDeliverySample[] {
  const cutoff = now - DIAGNOSTIC_WINDOW_MS;
  let start = 0;
  while (start + 1 < samples.length && samples[start + 1]!.receivedAt < cutoff) start += 1;
  return samples.slice(start);
}

function windowedTimestamps(samples: readonly number[], now: number): readonly number[] {
  const cutoff = now - DIAGNOSTIC_WINDOW_MS;
  let start = 0;
  while (start + 1 < samples.length && samples[start + 1]! < cutoff) start += 1;
  return samples.slice(start);
}

function gapStatistics(samples: readonly number[], now: number): GapStatistics | undefined {
  if (samples.length === 0) return undefined;
  const gaps = samples.slice(1).map((sample, index) => sample - samples[index]!);
  gaps.push(Math.max(0, now - samples.at(-1)!));
  return summarizeGaps(gaps);
}

function sourceGapStatistics(
  samples: readonly FrameDeliverySample[],
): GapStatistics | undefined {
  const gaps: number[] = [];
  for (let index = 1; index < samples.length; index += 1) {
    const gap = Number(
      samples[index]!.sourceTimestampNs - samples[index - 1]!.sourceTimestampNs,
    ) / 1_000_000;
    if (Number.isFinite(gap) && gap >= 0) gaps.push(gap);
  }
  return summarizeGaps(gaps);
}

function summarizeGaps(gaps: number[]): GapStatistics | undefined {
  if (gaps.length === 0) return undefined;
  const sorted = gaps.sort((left, right) => left - right);
  return {
    p95: sorted[Math.min(sorted.length - 1, Math.max(0, Math.ceil(sorted.length * 0.95) - 1))]!,
    maximum: sorted.at(-1)!,
  };
}

function setTileDiagnostics(
  element: HTMLElement,
  state: CameraTileState,
  diagnostics = streamDiagnostics(state),
): void {
  const receiveFps = diagnostics.fps?.toFixed(1) ?? "—";
  const renderFps = diagnostics.displayFps?.toFixed(1) ?? "—";
  const bandwidth = diagnostics.kibPerSecond === undefined
    ? "— KiB/s"
    : `${diagnostics.kibPerSecond.toFixed(1)} KiB/s`;
  const frameAge = formatCompactFrameAge(diagnostics.frameAgeMs);
  element.textContent = [
    `${receiveFps}\u00a0RX\u00a0fps`,
    `${renderFps}\u00a0render\u00a0fps`,
    bandwidth.replaceAll(" ", "\u00a0"),
    frameAge.replaceAll(" ", "\u00a0"),
  ].join(" · ");
  element.setAttribute(
    "aria-label",
    `Network receive rate ${formatRate(diagnostics.fps)}; render rate ${formatRate(diagnostics.displayFps)}; receive bandwidth ${formatBandwidth(diagnostics.kibPerSecond)}; displayed frame age ${frameAge}`,
  );
  const renderSize = diagnostics.displayWidth && diagnostics.displayHeight
    ? ` · display ${diagnostics.displayWidth}×${diagnostics.displayHeight}`
    : "";
  const renderCost = diagnostics.decodeMs === undefined
    ? ""
    : ` · queue ${formatDuration(diagnostics.queueMs)} · decode ${formatDuration(diagnostics.decodeMs)} · present ${formatDuration(diagnostics.presentMs)}`;
  element.title = `RX = network receive rate · render = frames drawn · ${formatFrameSize(diagnostics.averageFrameKib)}${renderSize}${renderCost} · ${diagnostics.supersededFrames} superseded (${diagnostics.queueOverflowFrames} queue overflow) · rolling five-second window`;
}

function setTileDiagnosticAttributes(
  tile: HTMLElement,
  state: CameraTileState,
  diagnostics = streamDiagnostics(state),
): void {
  setFiniteDataset(tile, "streamFps", diagnostics.fps);
  setFiniteDataset(tile, "displayFps", diagnostics.displayFps);
  setFiniteDataset(tile, "kibPerSecond", diagnostics.kibPerSecond);
  setFiniteDataset(tile, "averageFrameKib", diagnostics.averageFrameKib);
  setFiniteDataset(tile, "frameAgeMs", diagnostics.frameAgeMs);
  setFiniteDataset(tile, "queueMs", diagnostics.queueMs);
  setFiniteDataset(tile, "decodeMs", diagnostics.decodeMs);
  setFiniteDataset(tile, "presentMs", diagnostics.presentMs);
  setFiniteDataset(tile, "displayWidth", diagnostics.displayWidth);
  setFiniteDataset(tile, "displayHeight", diagnostics.displayHeight);
  setFiniteDataset(tile, "supersededFrames", diagnostics.supersededFrames);
  setFiniteDataset(tile, "queueOverflowFrames", diagnostics.queueOverflowFrames);
  if (diagnostics.renderer) tile.dataset.renderer = diagnostics.renderer;
  else delete tile.dataset.renderer;
}

function setFiniteDataset(
  element: HTMLElement,
  name: "streamFps"
    | "displayFps"
    | "kibPerSecond"
    | "averageFrameKib"
    | "frameAgeMs"
    | "queueMs"
    | "decodeMs"
    | "presentMs"
    | "displayWidth"
    | "displayHeight"
    | "supersededFrames"
    | "queueOverflowFrames",
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
  const displaySize = diagnostics.displayWidth && diagnostics.displayHeight
    ? ` · display ${diagnostics.displayWidth}×${diagnostics.displayHeight}`
    : "";
  const renderCost = diagnostics.decodeMs === undefined
    ? ""
    : ` · queue ${formatDuration(diagnostics.queueMs)} · decode ${formatDuration(diagnostics.decodeMs)} · present ${formatDuration(diagnostics.presentMs)}`;
  diagnosticFrameSize.textContent = `${formatFrameSize(diagnostics.averageFrameKib)}${displaySize}${renderCost}`;
  diagnosticFrameAge.textContent = formatFrameAge(diagnostics.frameAgeMs);
}

function showPresentedFrame(
  state: CameraTileState,
  presentation: JpegPresentation,
): void {
  if (cameras.get(state.candidate.peerId) !== state) return;
  const tile = cameraElement(state.candidate.peerId);
  const surface = state.frameSurface;
  const firstPresentation = !state.hasRenderedFrame;
  state.hasRenderedFrame = true;
  state.displayedTimestampNs = presentation.timestampNs;
  surface.hidden = false;
  surface.classList.toggle("dimmed", state.status !== "live");
  recordFrameDisplay(state);

  if (tile?.contains(surface)) {
    tile.dataset.status = state.status;
    tile.dataset.frameCount = String(state.received);
    // A decode can finish after the Stream ended. Keep the offline message and
    // Retry action rendered by markCameraEnded in that race.
    if (state.status === "live") {
      tile.querySelector<HTMLElement>(".tile-center")?.replaceChildren();
    }
  }
  scheduleLiveTelemetryRefresh(firstPresentation ? 0 : LIVE_TELEMETRY_INTERVAL_MS);
}

function scheduleLiveTelemetryRefresh(delayMs: number): void {
  if (liveTelemetryTimer !== undefined) return;
  liveTelemetryTimer = window.setTimeout(() => {
    liveTelemetryTimer = undefined;
    refreshLiveTelemetry();
  }, delayMs);
}

function refreshLiveTelemetry(): void {
  for (const state of cameras.values()) {
    if (!state.hasRenderedFrame) continue;
    const tile = cameraElement(state.candidate.peerId);
    if (!tile?.contains(state.frameSurface)) continue;
    const diagnostics = streamDiagnostics(state);
    tile.dataset.status = state.status;
    tile.dataset.frameCount = String(state.received);
    const time = tile.querySelector<HTMLElement>("[data-role='frame-time']");
    if (time && state.displayedTimestampNs !== undefined) {
      time.textContent = formatFrameTime(state.displayedTimestampNs);
    }
    const metrics = tile.querySelector<HTMLElement>("[data-role='stream-diagnostics']");
    if (metrics) setTileDiagnostics(metrics, state, diagnostics);
    const status = tile.querySelector<HTMLElement>(".feed-status");
    if (status) status.textContent = statusLabel(state);
    setTileDiagnosticAttributes(tile, state, diagnostics);
  }
  updateAggregateMetrics();
  const selected = selectedPeerId ? cameras.get(selectedPeerId) : undefined;
  if (selected) updateLiveDiagnostics(selected);
}

function revokeAfterPaint(frameUrl: string): void {
  requestAnimationFrame(() => requestAnimationFrame(() => URL.revokeObjectURL(frameUrl)));
}

function recordFrameDisplay(state: CameraTileState): void {
  const displayedAt = performance.now();
  state.totalRenderedFrames += 1;
  state.displaySamples.push(displayedAt);
  const cutoff = displayedAt - DIAGNOSTIC_WINDOW_MS;
  while (state.displaySamples.length > 2 && state.displaySamples[1]! < cutoff) {
    state.displaySamples.shift();
  }
  if (state.displaySamples.length > MAX_DIAGNOSTIC_SAMPLES) {
    state.displaySamples.splice(0, state.displaySamples.length - MAX_DIAGNOSTIC_SAMPLES);
  }
}

function sampleRate(samples: readonly number[], now: number): number | undefined {
  if (samples.length < 2) return undefined;
  const first = samples[0]!;
  return (samples.length - 1) / Math.max(0.001, (now - first) / 1_000);
}

function clearCameraSurface(state: CameraTileState): void {
  state.hasRenderedFrame = false;
  state.frameRenderer.clear();
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

function togglePerformanceRecording(): void {
  if (performanceCapture) {
    stopPerformanceRecording();
  } else {
    startPerformanceRecording();
  }
}

function startPerformanceRecording(): void {
  const running = mesh;
  if (!running || running.role !== "viewer" || performanceCapture) return;
  completedPerformanceReport = undefined;
  startEventLoopProbe();
  performanceCapture = new CameraPerformanceCapture({
    runtime: "web",
    platform: navigator.userAgent,
    domainId: running.domainId,
    localPeerId: running.peerId,
    columnCount: effectiveColumnCount(),
  });
  samplePerformance();
  performanceCaptureTimer = window.setInterval(() => {
    samplePerformance();
    renderPerformanceControls();
  }, CAMERA_PERFORMANCE_SAMPLE_INTERVAL_MS);
  record(`Performance recording started for ${cameras.size} camera(s)`);
  renderPerformanceControls();
}

function stopPerformanceRecording(openReport = true): void {
  const capture = performanceCapture;
  if (!capture) return;
  capture.recordEvent("Performance recording stopped");
  completedPerformanceReport = capture.finish(
    performanceSnapshots([...cameras.values()]),
    effectiveColumnCount(),
  );
  performanceCapture = undefined;
  if (performanceCaptureTimer !== undefined) window.clearInterval(performanceCaptureTimer);
  performanceCaptureTimer = undefined;
  stopEventLoopProbe();
  renderPerformanceControls();
  renderPerformanceReport();
  record(
    `Performance report ready with ${completedPerformanceReport.peers.length} camera(s)`,
  );
  if (openReport) showModal(performanceReportDialog);
}

function samplePerformance(states = [...cameras.values()]): void {
  performanceCapture?.sample(
    performanceSnapshots(states),
    effectiveColumnCount(),
  );
}

function performanceSnapshots(
  states: readonly CameraTileState[],
): CameraPerformanceSnapshot[] {
  const eventLoop = eventLoopDelayStatistics();
  return states.map((state) => {
    const diagnostics = streamDiagnostics(state, true);
    const profile = state.connection?.metadata.profile;
    return {
      peerId: state.candidate.peerId,
      name: state.name,
      runtime: state.connection?.metadata.info.appInstance ?? "unknown",
      status: state.sourcePaused ? "paused" : state.status,
      quality: profile?.quality,
      width: profile?.width,
      height: profile?.height,
      targetFps: profile?.rateHz,
      totalReceivedFrames: state.totalReceivedFrames,
      totalRenderedFrames: state.totalRenderedFrames,
      totalReceivedBytes: state.totalReceivedBytes,
      receiveFps: diagnostics.fps,
      renderFps: diagnostics.displayFps,
      kibPerSecond: diagnostics.kibPerSecond,
      frameAgeMs: diagnostics.frameAgeMs,
      sourceGapP95Ms: diagnostics.sourceGapP95Ms,
      sourceGapMaxMs: diagnostics.sourceGapMaxMs,
      receiveGapP95Ms: diagnostics.receiveGapP95Ms,
      receiveGapMaxMs: diagnostics.receiveGapMaxMs,
      renderGapP95Ms: diagnostics.renderGapP95Ms,
      renderGapMaxMs: diagnostics.renderGapMaxMs,
      renderer: diagnostics.renderer,
      pageVisibility: document.visibilityState,
      renderVisible: state.renderVisible,
      rendererEnabled: diagnostics.rendererEnabled,
      decodeInFlight: diagnostics.decodeInFlight,
      pendingFrames: diagnostics.pendingFrames,
      activeDecodes: diagnostics.activeDecodes,
      queuedRenderers: diagnostics.queuedRenderers,
      maximumActiveDecodes: diagnostics.maximumActiveDecodes,
      displayWidth: diagnostics.displayWidth,
      displayHeight: diagnostics.displayHeight,
      queueMs: diagnostics.queueMs,
      queueP95Ms: diagnostics.queueP95Ms,
      queueMaxMs: diagnostics.queueMaxMs,
      decodeMs: diagnostics.decodeMs,
      decodeP50Ms: diagnostics.decodeP50Ms,
      decodeP95Ms: diagnostics.decodeP95Ms,
      decodeMaxMs: diagnostics.decodeMaxMs,
      presentMs: diagnostics.presentMs,
      totalSupersededFrames: diagnostics.supersededFrames,
      totalQueueOverflowFrames: diagnostics.queueOverflowFrames,
      eventLoopDelayP95Ms: eventLoop?.p95,
      eventLoopDelayMaxMs: eventLoop?.maximum,
    };
  });
}

function startEventLoopProbe(): void {
  stopEventLoopProbe();
  eventLoopDelaySamples = [];
  scheduleEventLoopProbe();
}

function scheduleEventLoopProbe(): void {
  const expectedAt = performance.now() + EVENT_LOOP_PROBE_INTERVAL_MS;
  eventLoopProbeTimer = window.setTimeout(() => {
    const observedAt = performance.now();
    eventLoopDelaySamples.push({
      observedAt,
      delayMs: Math.max(0, observedAt - expectedAt),
    });
    pruneEventLoopDelaySamples(observedAt);
    scheduleEventLoopProbe();
  }, EVENT_LOOP_PROBE_INTERVAL_MS);
}

function stopEventLoopProbe(): void {
  if (eventLoopProbeTimer !== undefined) window.clearTimeout(eventLoopProbeTimer);
  eventLoopProbeTimer = undefined;
}

function eventLoopDelayStatistics(now = performance.now()): GapStatistics | undefined {
  pruneEventLoopDelaySamples(now);
  return summarizeGaps(eventLoopDelaySamples.map((sample) => sample.delayMs));
}

function pruneEventLoopDelaySamples(now: number): void {
  const cutoff = now - DIAGNOSTIC_WINDOW_MS;
  while (
    eventLoopDelaySamples.length > 1
    && eventLoopDelaySamples[0]!.observedAt < cutoff
  ) {
    eventLoopDelaySamples.shift();
  }
  if (eventLoopDelaySamples.length > MAX_DIAGNOSTIC_SAMPLES) {
    eventLoopDelaySamples.splice(
      0,
      eventLoopDelaySamples.length - MAX_DIAGNOSTIC_SAMPLES,
    );
  }
}

function renderPerformanceControls(): void {
  const capture = performanceCapture;
  recordPerformanceButton.classList.toggle("recording", capture !== undefined);
  recordPerformanceButton.setAttribute("aria-pressed", String(capture !== undefined));
  if (capture) {
    const seconds = Math.max(
      0,
      Math.floor((performance.now() - capture.startedAtMonotonicMs) / 1_000),
    );
    recordPerformanceLabel.textContent = `Stop · ${formatRecordingElapsed(seconds)}`;
    recordPerformanceButton.setAttribute(
      "aria-label",
      `Stop performance recording after ${seconds} seconds`,
    );
  } else {
    recordPerformanceLabel.textContent = "Record stats";
    recordPerformanceButton.setAttribute("aria-label", "Record performance statistics");
  }
  openPerformanceReportButton.hidden = completedPerformanceReport === undefined;
}

function openPerformanceReport(): void {
  if (!completedPerformanceReport) return;
  renderPerformanceReport();
  showModal(performanceReportDialog);
}

function renderPerformanceReport(): void {
  const report = completedPerformanceReport;
  if (!report) {
    performanceReportSummary.textContent = "No performance recording yet.";
    return;
  }
  const samples = report.peers.reduce((total, peer) => total + peer.summary.sampleCount, 0);
  const received = report.peers.reduce((total, peer) => total + peer.summary.receivedFrames, 0);
  const rendered = report.peers.reduce((total, peer) => total + peer.summary.renderedFrames, 0);
  const superseded = report.peers.reduce(
    (total, peer) => total + (peer.summary.supersededFrames ?? 0),
    0,
  );
  const queueOverflow = report.peers.reduce(
    (total, peer) => total + (peer.summary.queueOverflowFrames ?? 0),
    0,
  );
  const ratio = received > 0 ? `${(rendered / received * 100).toFixed(1)}% rendered` : "no frames";
  performanceReportSummary.textContent = [
    `${report.peers.length} camera(s) · ${(report.durationMs / 1_000).toFixed(1)} seconds · ${samples} samples`,
    `${received} received · ${rendered} rendered · ${superseded} superseded (${queueOverflow} queue overflow) · ${ratio}`,
  ].join("\n");
}

async function copyPerformanceReport(): Promise<void> {
  const performanceReport = completedPerformanceReport;
  if (!performanceReport) return;
  try {
    await navigator.clipboard.writeText(serializeCameraPerformanceReport(performanceReport));
    showToast("Performance report copied");
  } catch (error) {
    report(error);
  }
}

function downloadPerformanceReport(): void {
  const report = completedPerformanceReport;
  if (!report) return;
  const url = URL.createObjectURL(new Blob(
    [serializeCameraPerformanceReport(report)],
    { type: "application/json" },
  ));
  const link = document.createElement("a");
  link.href = url;
  link.download = cameraPerformanceReportFilename(report);
  link.click();
  revokeAfterPaint(url);
  showToast("Performance report downloaded");
}

function formatRecordingElapsed(seconds: number): string {
  const minutes = Math.floor(seconds / 60);
  return `${minutes}:${String(seconds % 60).padStart(2, "0")}`;
}

async function stopPeer(): Promise<void> {
  const running = mesh;
  if (!running) return;
  if (performanceCapture) stopPerformanceRecording(false);
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
    for (const candidate of [
      addDialog,
      diagnosticsDialog,
      snapshotDialog,
      cameraActionsDialog,
      performanceReportDialog,
    ]) {
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
  performanceCapture?.recordEvent(message);
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

function formatCompactFrameAge(value: number | undefined): string {
  if (value === undefined) return "— ms";
  const rounded = Math.round(value);
  return `${rounded >= 0 ? rounded : `−${Math.abs(rounded)}`} ms`;
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

function selectedAddQuality(): CameraQualityTier {
  return isCameraQualityTier(addCameraQuality.value)
    ? addCameraQuality.value
    : preferredQualityForWall();
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
