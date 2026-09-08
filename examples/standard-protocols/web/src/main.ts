import init, { AukiDiscoveryMode, AukiUserSession } from "../pkg-web/auki_sdk_web.js";
import {
  BrowserPlayground,
  type DiscoveredPeer,
  type PeerCard,
  type ProbeResult,
} from "./playground.js";

const get = <T extends HTMLElement>(id: string): T => document.querySelector<T>(`#${id}`)!;
const input = (id: string): HTMLInputElement => get<HTMLInputElement>(id);
const loginForm = get<HTMLFormElement>("login");
const startForm = get<HTMLFormElement>("start");
const probeForm = get<HTMLFormElement>("probe");
const discoverForm = get<HTMLFormElement>("discover");
const domain = get<HTMLSelectElement>("domain");
const discoveryMode = get<HTMLSelectElement>("discovery-mode");
const protocolFilter = get<HTMLSelectElement>("protocol-filter");
const discoveredPeer = get<HTMLSelectElement>("discovered-peer");
const discoveredDetails = get<HTMLElement>("discovered-details");
const local = get<HTMLElement>("local");
const remoteCard = get<HTMLTextAreaElement>("remote-card");
const log = get<HTMLElement>("log");
let session: AukiUserSession | undefined;
let playground: BrowserPlayground | undefined;
let pendingPlayground: Promise<BrowserPlayground> | undefined;
let discoveredPeers: DiscoveredPeer[] = [];
let lifecycleGeneration = 0;
let discoveryRequest = 0;

await init();
get<HTMLButtonElement>("login-button").disabled = false;
write("Runtime ready");

loginForm.onsubmit = (event) => { event.preventDefault(); void login(); };
startForm.onsubmit = (event) => { event.preventDefault(); void startPeer(); };
probeForm.onsubmit = (event) => { event.preventDefault(); void probe(); };
discoverForm.onsubmit = (event) => { event.preventDefault(); void refreshDiscovery(); };
discoveredPeer.onchange = showSelectedCandidate;
get<HTMLButtonElement>("copy").onclick = () => void navigator.clipboard.writeText(local.textContent ?? "");
get<HTMLButtonElement>("stop-button").onclick = () => void stopPeer().catch(write);
get<HTMLButtonElement>("probe-discovered-button").onclick = () => void probeSelected();

async function login(): Promise<void> {
  const button = get<HTMLButtonElement>("login-button");
  button.disabled = true;
  let authenticated: AukiUserSession | undefined;
  try {
    authenticated = await AukiUserSession.loginDev(input("email").value, input("password").value);
    const choices = await authenticated.accessibleDomains();
    const options = choices.map((choice) => {
      try {
        const option = document.createElement("option");
        option.value = choice.id;
        option.textContent = choice.name ? `${choice.name} — ${choice.id}` : choice.id;
        return option;
      } finally { choice.free(); }
    });
    if (!options.length) throw new Error("This User has no accessible Domains");
    domain.replaceChildren(...options);
    session = authenticated;
    authenticated = undefined;
    input("email").disabled = true;
    input("password").disabled = true;
    startForm.hidden = false;
    write("Choose a Domain and start the peer");
  } catch (error) {
    button.disabled = false;
    write(error);
  } finally {
    input("password").value = "";
    authenticated?.free();
  }
}

async function startPeer(): Promise<void> {
  const authenticated = session;
  if (!authenticated) return;
  get<HTMLButtonElement>("start-button").disabled = true;
  try {
    playground = await startOwnedPlayground(
      authenticated,
      domain.value,
      "browser-playground",
      modeFromValue(discoveryMode.value),
      write,
    );
    lifecycleGeneration += 1;
    session = undefined;
    authenticated.free();
    const card = playground.card();
    showCard(card);
    showProtocolFilters(card.protocols);
    startForm.hidden = true;
    get<HTMLElement>("discovery-section").hidden = false;
    get<HTMLButtonElement>("discover-button").disabled = false;
    get<HTMLButtonElement>("probe-button").disabled = false;
    get<HTMLButtonElement>("stop-button").disabled = false;
    write("All six standard protocols are serving");
  } catch (error) {
    get<HTMLButtonElement>("start-button").disabled = false;
    write(error);
  }
}

async function refreshDiscovery(): Promise<void> {
  const running = playground;
  if (!running) return;
  const generation = lifecycleGeneration;
  const request = ++discoveryRequest;
  const button = get<HTMLButtonElement>("discover-button");
  button.disabled = true;
  try {
    const candidates = await running.discover(protocolFilter.value || undefined);
    if (
      playground !== running
      || lifecycleGeneration !== generation
      || discoveryRequest !== request
    ) return;
    discoveredPeers = candidates;
    renderCandidates();
    write(`Discovery returned ${discoveredPeers.length} current candidate(s)`);
  } catch (error) {
    if (
      playground === running
      && lifecycleGeneration === generation
      && discoveryRequest === request
    ) write(error);
  } finally {
    if (
      playground === running
      && lifecycleGeneration === generation
      && discoveryRequest === request
    ) button.disabled = false;
  }
}

async function probeSelected(): Promise<void> {
  const running = playground;
  const candidate = discoveredPeers.find((value) => value.peerId === discoveredPeer.value);
  if (!running || !candidate) return;
  const generation = lifecycleGeneration;
  const button = get<HTMLButtonElement>("probe-discovered-button");
  button.disabled = true;
  try {
    const result = await running.probeDiscovered(candidate);
    if (playground !== running || lifecycleGeneration !== generation) return;
    write(
      result.ok
        ? `All six protocols passed against discovered peer ${candidate.peerId}`
        : JSON.stringify(result.errors),
    );
  } catch (error) {
    if (playground === running && lifecycleGeneration === generation) write(error);
  } finally {
    if (playground === running) button.disabled = false;
  }
}

async function probe(): Promise<void> {
  const running = playground;
  if (!running) return;
  const generation = lifecycleGeneration;
  const button = get<HTMLButtonElement>("probe-button");
  button.disabled = true;
  try {
    const target = JSON.parse(remoteCard.value) as PeerCard;
    const result = await running.probeAll(target);
    if (playground !== running || lifecycleGeneration !== generation) return;
    write(result.ok ? `All six protocols passed against ${target.peerId}` : JSON.stringify(result.errors));
  } catch (error) {
    if (playground === running && lifecycleGeneration === generation) write(error);
  } finally {
    if (playground === running) button.disabled = false;
  }
}

async function stopPeer(): Promise<void> {
  let running = playground;
  playground = undefined;
  const pending = pendingPlayground;
  pendingPlayground = undefined;
  if (!running && pending) {
    try {
      running = await pending;
    } catch {
      return;
    }
  }
  if (!running) return;
  lifecycleGeneration += 1;
  discoveryRequest += 1;
  get<HTMLButtonElement>("probe-button").disabled = true;
  get<HTMLButtonElement>("discover-button").disabled = true;
  get<HTMLButtonElement>("probe-discovered-button").disabled = true;
  get<HTMLButtonElement>("stop-button").disabled = true;
  try {
    await running.close();
  } finally {
    local.textContent = "Peer stopped";
    get<HTMLElement>("discovery-section").hidden = true;
    discoveredPeers = [];
    renderCandidates();
    for (const key of Object.keys(local.dataset)) delete local.dataset[key];
    input("email").disabled = false;
    input("password").disabled = false;
    get<HTMLButtonElement>("login-button").disabled = false;
    write("Peer stopped");
  }
}

async function startOwnedPlayground(
  authenticated: AukiUserSession,
  domainId: string,
  nodeName: string,
  mode: AukiDiscoveryMode,
  onEvent: (message: string) => void,
): Promise<BrowserPlayground> {
  const starting = BrowserPlayground.start(
    authenticated,
    domainId,
    nodeName,
    mode,
    onEvent,
  );
  pendingPlayground = starting;
  try {
    const started = await starting;
    if (pendingPlayground !== starting) {
      await started.close();
      throw new Error("Peer startup was stopped");
    }
    playground = started;
    pendingPlayground = undefined;
    return started;
  } catch (error) {
    if (pendingPlayground === starting) pendingPlayground = undefined;
    throw error;
  }
}

function showProtocolFilters(protocols: string[]): void {
  const all = document.createElement("option");
  all.value = "";
  all.textContent = "All advertised peers";
  const exact = protocols.map((protocol) => {
    const option = document.createElement("option");
    option.value = protocol;
    option.textContent = protocol;
    return option;
  });
  protocolFilter.replaceChildren(all, ...exact);
}

function renderCandidates(): void {
  const running = playground;
  const options = discoveredPeers.map((candidate) => {
    const option = document.createElement("option");
    option.value = candidate.peerId;
    const compatible = running?.canProbeDiscovered(candidate) === true;
    option.disabled = !compatible;
    option.textContent = `${candidate.peerId} — ${candidate.servedProtocols.length} protocol(s)${compatible ? "" : " — not probeable here"}`;
    return option;
  });
  if (!options.length) {
    const empty = document.createElement("option");
    empty.value = "";
    empty.textContent = "No candidates found";
    discoveredPeer.replaceChildren(empty);
  } else {
    discoveredPeer.replaceChildren(...options);
    const selected = discoveredPeers.find(
      (candidate) => candidate.peerId === discoveredPeer.value
        && running?.canProbeDiscovered(candidate),
    );
    if (!selected) {
      discoveredPeer.value = discoveredPeers.find(
        (candidate) => running?.canProbeDiscovered(candidate),
      )?.peerId ?? "";
    }
  }
  discoveredPeer.disabled = !options.length;
  showSelectedCandidate();
}

function showSelectedCandidate(): void {
  const candidate = discoveredPeers.find((value) => value.peerId === discoveredPeer.value);
  discoveredDetails.textContent = candidate
    ? JSON.stringify(candidate, null, 2)
    : "No discovered peer selected.";
  get<HTMLButtonElement>("probe-discovered-button").disabled =
    !candidate || playground?.canProbeDiscovered(candidate) !== true;
}

function modeFromValue(value: string): AukiDiscoveryMode {
  switch (value) {
    case "discover_only": return AukiDiscoveryMode.DiscoverOnly;
    case "discover_and_advertise": return AukiDiscoveryMode.DiscoverAndAdvertise;
    default: throw new Error("Select an explicit DDS discovery mode");
  }
}

function showCard(card: PeerCard): void {
  local.textContent = JSON.stringify(card, null, 2);
  local.dataset.peerId = card.peerId;
  local.dataset.domainId = card.domainId;
  local.dataset.wssRoute = card.routes.wss;
  local.dataset.tcpRoute = card.routes.tcp;
  get<HTMLButtonElement>("copy").disabled = false;
}

function write(value: unknown): void {
  const line = value instanceof Error ? value.message : String(value);
  log.textContent = `${log.textContent}${line}\n`.slice(-16_384);
}

interface E2eApi {
  start(input: {
    email: string;
    password: string;
    domainId: string;
    label: string;
    discoveryMode?: "discover_only" | "discover_and_advertise";
  }): Promise<PeerCard>;
  card(): PeerCard;
  discover(protocolId?: string): Promise<DiscoveredPeer[]>;
  probeDiscovered(target: DiscoveredPeer): Promise<ProbeResult>;
  probeAll(target: PeerCard): Promise<ProbeResult>;
  stop(): Promise<void>;
}

declare global { interface Window { aukiE2e: E2eApi; } }

window.aukiE2e = {
  async start(input): Promise<PeerCard> {
    if (playground) await stopPeer();
    const authenticated = await AukiUserSession.loginDev(input.email, input.password);
    try {
      playground = await startOwnedPlayground(
        authenticated,
        input.domainId,
        input.label,
        modeFromValue(input.discoveryMode ?? "discover_and_advertise"),
        write,
      );
    } finally {
      authenticated.free();
    }
    const card = playground.card();
    showCard(card);
    return card;
  },
  card(): PeerCard {
    if (!playground) throw new Error("browser playground is not running");
    return playground.card();
  },
  discover(protocolId): Promise<DiscoveredPeer[]> {
    if (!playground) throw new Error("browser playground is not running");
    return playground.discover(protocolId);
  },
  probeDiscovered(target): Promise<ProbeResult> {
    if (!playground) throw new Error("browser playground is not running");
    return playground.probeDiscovered(target);
  },
  probeAll(target): Promise<ProbeResult> {
    if (!playground) throw new Error("browser playground is not running");
    return playground.probeAll(target);
  },
  stop: stopPeer,
};
