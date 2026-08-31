import init, {
  BrowserEchoPeer,
  BrowserUserSession,
} from "../pkg-web/auki_portable_echo_web.js";
import "./style.css";

const PEER_CARD_VERSION = 1;
const MAX_EVENTS = 80;
const textEncoder = new TextEncoder();
const textDecoder = new TextDecoder();

interface DomainOption {
  id: string;
  name?: string;
  description?: string;
}

interface PeerCard {
  version: typeof PEER_CARD_VERSION;
  domainId: string;
  peerId: string;
  protocols: string[];
  routes: {
    wss: string;
    tcp?: string;
  };
}

interface EchoTarget {
  domainId: string;
  peerId: string;
  protocol: string;
}

type EventDirection = "inbound" | "outbound" | "system";
type StatusTone = "idle" | "busy" | "ready" | "error";

const ui = {
  status: element<HTMLElement>("app-status"),
  loginForm: element<HTMLFormElement>("login-form"),
  email: element<HTMLInputElement>("email"),
  password: element<HTMLInputElement>("password"),
  loginButton: element<HTMLButtonElement>("login-button"),
  domainPanel: element<HTMLElement>("domain-panel"),
  domainSelect: element<HTMLSelectElement>("domain-select"),
  startPeer: element<HTMLButtonElement>("start-peer"),
  peerPanel: element<HTMLElement>("peer-panel"),
  localPeerId: element<HTMLElement>("local-peer-id"),
  localDomainId: element<HTMLElement>("local-domain-id"),
  peerCard: element<HTMLTextAreaElement>("peer-card"),
  copyPeerCard: element<HTMLButtonElement>("copy-peer-card"),
  connectPanel: element<HTMLElement>("connect-panel"),
  remotePeer: element<HTMLTextAreaElement>("remote-peer"),
  message: element<HTMLInputElement>("echo-message"),
  sendEcho: element<HTMLButtonElement>("send-echo"),
  stopPeer: element<HTMLButtonElement>("stop-peer"),
  eventLog: element<HTMLOListElement>("event-log"),
};

let userSession: BrowserUserSession | undefined;
let runningPeer: BrowserEchoPeer | undefined;
let localCard: PeerCard | undefined;
let lifecycleGeneration = 0;
let peerStartup: Promise<void> | undefined;
let receiveGeneration = 0;
let receiveTask: Promise<void> | undefined;
const activeSends = new Set<Promise<void>>();

bindAction(ui.domainPanel, ui.startPeer, beginPeerStartup);
bindAction(ui.connectPanel, ui.sendEcho, sendEcho);
ui.loginForm.addEventListener("submit", (event) => {
  event.preventDefault();
  void login();
});
ui.copyPeerCard.addEventListener("click", () => void copyPeerCard());
ui.stopPeer.addEventListener("click", () => void stopPeer());
window.addEventListener("pagehide", releasePageResources);
window.addEventListener("pageshow", (event) => {
  if (event.persisted) {
    window.location.reload();
  }
});

void boot();

async function boot(): Promise<void> {
  setStatus("Loading the Rust/Wasm runtime…", "busy");
  try {
    await init();
    ui.loginButton.disabled = false;
    setStatus("Ready. Log in to list your accessible Domains.", "idle");
  } catch (error) {
    setStatus(`Runtime failed to load: ${errorMessage(error)}`, "error");
  }
}

async function login(): Promise<void> {
  if (userSession || runningPeer) {
    return;
  }
  const email = ui.email.value.trim();
  const password = ui.password.value;
  if (!email || !password) {
    setStatus("Enter both username and password.", "error");
    return;
  }

  setLoginBusy(true);
  setStatus("Authenticating and loading accessible Domains…", "busy");
  const generation = lifecycleGeneration;
  let authenticated: BrowserUserSession | undefined;
  try {
    ui.password.value = "";
    authenticated = await BrowserUserSession.loginDev(email, password);
    if (generation !== lifecycleGeneration) {
      return;
    }
    const domains = await readDomains(authenticated);
    if (generation !== lifecycleGeneration) {
      return;
    }
    if (domains.length === 0) {
      throw new Error("This User has no accessible Domains.");
    }
    populateDomains(domains);
    userSession = authenticated;
    authenticated = undefined;
    ui.email.disabled = true;
    ui.password.disabled = true;
    setPanelBusy(ui.domainPanel, false);
    ui.domainPanel.hidden = false;
    ui.startPeer.focus();
    setStatus("Authenticated. Choose a Domain for this ephemeral peer.", "ready");
  } catch (error) {
    if (generation === lifecycleGeneration) {
      setStatus(`Login failed: ${errorMessage(error)}`, "error");
      setLoginBusy(false);
    }
  } finally {
    ui.password.value = "";
    authenticated?.free();
  }
}

async function readDomains(session: BrowserUserSession): Promise<DomainOption[]> {
  const bindings = await session.accessibleDomains();
  const domains: DomainOption[] = [];
  for (const binding of bindings) {
    try {
      domains.push({
        id: binding.id,
        name: binding.name,
        description: binding.description,
      });
    } finally {
      binding.free();
    }
  }
  return domains;
}

function populateDomains(domains: DomainOption[]): void {
  const options = domains.map((domain) => {
    const option = document.createElement("option");
    option.value = domain.id;
    option.textContent = domain.name ? `${domain.name} — ${domain.id}` : domain.id;
    option.title = domain.description ?? domain.id;
    return option;
  });
  ui.domainSelect.replaceChildren(...options);
}

function beginPeerStartup(): Promise<void> {
  if (peerStartup) {
    return peerStartup;
  }
  const generation = lifecycleGeneration;
  let tracked: Promise<void>;
  tracked = startSelectedPeer(generation).finally(() => {
    if (peerStartup === tracked) {
      peerStartup = undefined;
    }
  });
  peerStartup = tracked;
  return tracked;
}

async function startSelectedPeer(generation: number): Promise<void> {
  const session = userSession;
  if (!session || runningPeer) {
    return;
  }
  const domainId = ui.domainSelect.value;
  if (!domainId) {
    setStatus("Choose a Domain first.", "error");
    return;
  }

  setPanelBusy(ui.domainPanel, true);
  setStatus("Authorizing a fresh Peer ID and acquiring its relay…", "busy");
  let started: BrowserEchoPeer | undefined;
  try {
    const peer = await session.startPeer(domainId);
    started = peer;
    if (generation !== lifecycleGeneration) {
      if (userSession === session) {
        userSession = undefined;
      }
      session.free();
      await peer.shutdown().catch(() => {});
      peer.free();
      started = undefined;
      return;
    }
    runningPeer = peer;
    localCard = peerCard(peer);
    renderPeer(localCard);
    started = undefined;
    userSession = undefined;
    session.free();
    ui.domainPanel.hidden = true;
    ui.peerPanel.hidden = false;
    ui.connectPanel.hidden = false;
    ui.stopPeer.hidden = false;
    ui.copyPeerCard.disabled = false;
    ui.stopPeer.disabled = false;
    setPanelBusy(ui.connectPanel, false);
    ui.remotePeer.focus();
    setStatus("Peer ready. Share its card or connect to another Peer ID.", "ready");
    const receiveLoopGeneration = ++receiveGeneration;
    const task = receiveEchoes(peer, receiveLoopGeneration);
    receiveTask = task;
    void task
      .finally(() => {
        if (receiveTask === task) {
          receiveTask = undefined;
        }
      })
      .catch(() => {});
  } catch (error) {
    if (started) {
      await started.shutdown().catch(() => {});
      started.free();
    }
    if (generation !== lifecycleGeneration) {
      if (userSession === session) {
        userSession = undefined;
      }
      session.free();
      return;
    }
    setStatus(`Peer startup failed: ${errorMessage(error)}`, "error");
    setPanelBusy(ui.domainPanel, false);
  }
}

function peerCard(peer: BrowserEchoPeer): PeerCard {
  const routes: PeerCard["routes"] = { wss: peer.wssRoute };
  if (peer.tcpRoute) {
    routes.tcp = peer.tcpRoute;
  }
  return {
    version: PEER_CARD_VERSION,
    domainId: peer.domainId,
    peerId: peer.peerId,
    protocols: [peer.protocol],
    routes,
  };
}

function renderPeer(card: PeerCard): void {
  ui.localPeerId.textContent = card.peerId;
  ui.localDomainId.textContent = card.domainId;
  ui.peerCard.value = JSON.stringify(card, null, 2);
}

async function copyPeerCard(): Promise<void> {
  const peer = runningPeer;
  if (!localCard || !peer) {
    return;
  }
  const card = ui.peerCard.value;
  try {
    await navigator.clipboard.writeText(card);
    if (runningPeer === peer) {
      setStatus("Public Peer Card copied.", "ready");
    }
  } catch (error) {
    if (runningPeer === peer) {
      setStatus(`Could not copy the Peer Card: ${errorMessage(error)}`, "error");
    }
  }
}

async function sendEcho(): Promise<void> {
  const task = sendOneEcho();
  activeSends.add(task);
  try {
    await task;
  } finally {
    activeSends.delete(task);
  }
}

async function sendOneEcho(): Promise<void> {
  const peer = runningPeer;
  const card = localCard;
  if (!peer || !card) {
    return;
  }
  const message = ui.message.value;
  if (!message) {
    setStatus("Enter a message to echo.", "error");
    return;
  }

  let target: EchoTarget;
  try {
    target = parseTarget(ui.remotePeer.value, card);
  } catch (error) {
    setStatus(`Invalid remote peer: ${errorMessage(error)}`, "error");
    return;
  }

  setPanelBusy(ui.connectPanel, true);
  setStatus(`Opening an authenticated echo stream to ${target.peerId}…`, "busy");
  try {
    const receipt = await peer.sendEcho(
      target.domainId,
      target.peerId,
      target.protocol,
      textEncoder.encode(message),
    );
    try {
      if (runningPeer !== peer) {
        return;
      }
      appendEvent("outbound", receipt.remotePeerId, textDecoder.decode(receipt.payload));
    } finally {
      receipt.free();
    }
    ui.message.value = "";
    setStatus("Echo response validated and the route was closed.", "ready");
  } catch (error) {
    if (runningPeer === peer) {
      setStatus(`Echo failed: ${errorMessage(error)}`, "error");
    }
  } finally {
    if (runningPeer === peer) {
      setPanelBusy(ui.connectPanel, false);
    }
  }
}

function parseTarget(raw: string, local: PeerCard): EchoTarget {
  const input = raw.trim();
  if (!input) {
    throw new Error("paste a Peer ID or Peer Card");
  }
  if (!input.startsWith("{")) {
    return {
      domainId: local.domainId,
      peerId: input,
      protocol: local.protocols[0],
    };
  }

  const candidate: unknown = JSON.parse(input);
  if (!isRecord(candidate) || candidate.version !== PEER_CARD_VERSION) {
    throw new Error(`expected Peer Card version ${PEER_CARD_VERSION}`);
  }
  if (typeof candidate.domainId !== "string" || typeof candidate.peerId !== "string") {
    throw new Error("Peer Card requires domainId and peerId strings");
  }
  if (!Array.isArray(candidate.protocols) || !candidate.protocols.includes(local.protocols[0])) {
    throw new Error(`Peer Card does not advertise ${local.protocols[0]}`);
  }
  return {
    domainId: candidate.domainId,
    peerId: candidate.peerId,
    protocol: local.protocols[0],
  };
}

async function receiveEchoes(peer: BrowserEchoPeer, generation: number): Promise<void> {
  let consecutiveFailures = 0;
  while (isActive(peer, generation)) {
    try {
      const receipt = await peer.serveOnce();
      try {
        if (!isActive(peer, generation)) {
          return;
        }
        appendEvent("inbound", receipt.remotePeerId, textDecoder.decode(receipt.payload));
      } finally {
        receipt.free();
      }
      consecutiveFailures = 0;
    } catch (error) {
      if (!isActive(peer, generation)) {
        return;
      }
      consecutiveFailures += 1;
      appendEvent("system", "receive loop", errorMessage(error));
      if (consecutiveFailures >= 3) {
        setStatus("Inbound serving paused after repeated failures. Stop and restart the peer.", "error");
        return;
      }
      await delay(500);
    }
  }
}

function isActive(peer: BrowserEchoPeer, generation: number): boolean {
  return runningPeer === peer && receiveGeneration === generation;
}

async function stopPeer(): Promise<void> {
  const peer = runningPeer;
  if (!peer) {
    return;
  }
  runningPeer = undefined;
  localCard = undefined;
  receiveGeneration += 1;
  const pendingReceive = receiveTask;
  setPanelBusy(ui.connectPanel, true);
  ui.stopPeer.disabled = true;
  setStatus("Stopping the peer and releasing its relay booking…", "busy");
  try {
    await peer.shutdown();
    setStatus("Peer stopped. Log in again to start a fresh Peer ID.", "idle");
  } catch (error) {
    setStatus(`Peer stopped with incomplete cleanup: ${errorMessage(error)}`, "error");
  } finally {
    await Promise.allSettled([
      ...(pendingReceive ? [pendingReceive] : []),
      ...activeSends,
    ]);
    peer.free();
    resetToLogin();
  }
}

function resetToLogin(): void {
  userSession?.free();
  userSession = undefined;
  ui.peerPanel.hidden = true;
  ui.connectPanel.hidden = true;
  ui.domainPanel.hidden = true;
  ui.stopPeer.hidden = true;
  ui.stopPeer.disabled = false;
  ui.peerCard.value = "";
  ui.copyPeerCard.disabled = true;
  ui.remotePeer.value = "";
  ui.message.value = "";
  ui.email.disabled = false;
  ui.password.disabled = false;
  setLoginBusy(false);
}

function releasePageResources(): void {
  lifecycleGeneration += 1;
  receiveGeneration += 1;
  const session = userSession;
  userSession = undefined;
  if (!peerStartup) {
    session?.free();
  }
  const peer = runningPeer;
  runningPeer = undefined;
  if (peer) {
    const pending = [
      ...(receiveTask ? [receiveTask] : []),
      ...activeSends,
    ];
    void peer
      .shutdown()
      .catch(() => {})
      .then(() => Promise.allSettled(pending))
      .finally(() => peer.free());
  }
}

function appendEvent(direction: EventDirection, remotePeerId: string, message: string): void {
  const item = document.createElement("li");
  item.className = `event event--${direction}`;
  item.dataset.direction = direction;

  const heading = document.createElement("div");
  heading.className = "event__heading";
  const label = document.createElement("strong");
  label.textContent = direction;
  const time = document.createElement("time");
  time.dateTime = new Date().toISOString();
  time.textContent = new Date().toLocaleTimeString();
  heading.append(label, time);

  const peer = document.createElement("code");
  peer.className = "event__peer";
  peer.textContent = remotePeerId;
  const payload = document.createElement("p");
  payload.textContent = message;
  item.append(heading, peer, payload);
  ui.eventLog.querySelector(".empty-log")?.remove();
  ui.eventLog.prepend(item);
  while (ui.eventLog.children.length > MAX_EVENTS) {
    ui.eventLog.lastElementChild?.remove();
  }
}

function setStatus(message: string, tone: StatusTone): void {
  ui.status.textContent = message;
  ui.status.dataset.tone = tone;
}

function setLoginBusy(busy: boolean): void {
  ui.loginButton.disabled = busy;
  if (!userSession && !runningPeer) {
    ui.email.disabled = busy;
    ui.password.disabled = busy;
  }
}

function setPanelBusy(panel: HTMLElement, busy: boolean): void {
  for (const control of panel.querySelectorAll<HTMLInputElement | HTMLButtonElement | HTMLSelectElement | HTMLTextAreaElement>(
    "input, button, select, textarea",
  )) {
    control.disabled = busy;
  }
}

function bindAction(panel: HTMLElement, button: HTMLButtonElement, action: () => Promise<void>): void {
  if (panel instanceof HTMLFormElement) {
    panel.addEventListener("submit", (event) => {
      event.preventDefault();
      void action();
    });
  } else {
    button.addEventListener("click", () => void action());
  }
}

function element<T extends HTMLElement>(id: string): T {
  const value = document.getElementById(id);
  if (!value) {
    throw new Error(`missing required element #${id}`);
  }
  return value as T;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function delay(milliseconds: number): Promise<void> {
  return new Promise((resolve) => window.setTimeout(resolve, milliseconds));
}
