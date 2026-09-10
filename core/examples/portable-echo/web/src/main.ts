import init, {
  AukiDiscoveryMode,
  AukiEcho,
  AukiEchoClient,
  AukiPeer,
  AukiPeerReachabilityMode,
  AukiUserSession,
} from "../pkg-web/auki_portable_echo_web.js";

const get = <T extends HTMLElement>(id: string): T => document.querySelector<T>(`#${id}`)!;
const input = (id: string): HTMLInputElement => get<HTMLInputElement>(id);
const login = get<HTMLFormElement>("login");
const start = get<HTMLFormElement>("start");
const domain = get<HTMLSelectElement>("domain");
const reachabilityMode = get<HTMLSelectElement>("reachability-mode");
const discoveryMode = get<HTMLSelectElement>("discovery-mode");
const local = get<HTMLElement>("local");
const discovery = get<HTMLFieldSetElement>("discovery");
const refreshButton = get<HTMLButtonElement>("refresh-button");
const candidates = get<HTMLSelectElement>("candidates");
const useCandidateButton = get<HTMLButtonElement>("use-candidate-button");
const sendButton = get<HTMLButtonElement>("send-button");
const stopButton = get<HTMLButtonElement>("stop-button");
const log = get<HTMLElement>("log");
let session: AukiUserSession | undefined;
let peer: AukiPeer | undefined;
let echo: AukiEcho | AukiEchoClient | undefined;
let echoEndpoint: AukiEcho | undefined;
let receiving: Promise<void> | undefined;
let sending: Promise<void> | undefined;
let refreshing: Promise<void> | undefined;
let discoveredRoutes = new Map<string, string[]>();

await init();
get<HTMLButtonElement>("login-button").disabled = false;
write("Runtime ready");
login.onsubmit = (event) => (event.preventDefault(), void authenticate());
start.onsubmit = (event) => (event.preventDefault(), void startPeer());
get<HTMLFormElement>("send").onsubmit = (event) => {
  event.preventDefault();
  if (!sending) sending = sendEcho().finally(() => { sending = undefined; });
};
stopButton.onclick = () => { void stopPeer().catch(write); };
refreshButton.onclick = () => {
  if (!refreshing) refreshing = refreshDiscovery().finally(() => { refreshing = undefined; });
};
candidates.onchange = () => { useCandidateButton.disabled = !selectedCandidate(); };
useCandidateButton.onclick = () => { useSelectedCandidate(); };
reachabilityMode.onchange = syncReachabilityControls;
syncReachabilityControls();

async function authenticate(): Promise<void> {
  const button = get<HTMLButtonElement>("login-button");
  let authenticated: AukiUserSession | undefined;
  button.disabled = true;
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
    input("email").disabled = input("password").disabled = true;
    get<HTMLButtonElement>("start-button").disabled = false;
    start.hidden = false;
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
  const button = get<HTMLButtonElement>("start-button");
  if (!authenticated) return;
  button.disabled = true;
  try {
    const started = await authenticated.startPeerWithDiscovery(
      domain.value,
      selectedDiscoveryMode(),
      selectedReachabilityMode(),
    );
    let adapter: AukiEcho | AukiEchoClient;
    let mounted: AukiEcho | undefined;
    try {
      if (started.relayBacked) {
        mounted = new AukiEcho(started);
        adapter = mounted;
      } else {
        adapter = new AukiEchoClient(started);
      }
    }
    catch (error) { try { await started.shutdown(); } finally { started.free(); } throw error; }
    session = undefined;
    authenticated.free();
    peer = started;
    echo = adapter;
    echoEndpoint = mounted;
    local.dataset.peerId = started.peerId;
    local.dataset.domainId = started.domainId;
    local.dataset.relayBacked = String(started.relayBacked);
    const wssRoute = started.wssRoute;
    const tcpRoute = started.tcpRoute;
    if (wssRoute) local.dataset.wssRoute = wssRoute;
    if (tcpRoute) local.dataset.tcpRoute = tcpRoute;
    local.textContent = started.relayBacked
      ? `Peer ID: ${started.peerId}\nReachability: relay-backed`
        + `\nWSS route: ${wssRoute}\nTCP route: ${tcpRoute}`
      : `Peer ID: ${started.peerId}\nReachability: outbound only\nRelay booking: disabled`;
    start.hidden = true;
    discovery.hidden = false;
    refreshButton.disabled = false;
    sendButton.disabled = stopButton.disabled = false;
    if (mounted) receiving = receiveEchoes(mounted);
    void started.waitStopped().catch((error) => {
      if (peer === started) {
        write(error);
        void stopPeer().catch(write);
      }
    });
  } catch (error) {
    button.disabled = false;
    write(error);
  }
}

async function refreshDiscovery(): Promise<void> {
  const running = peer;
  const adapter = echo;
  if (!running || !adapter) return;
  refreshButton.disabled = useCandidateButton.disabled = true;
  try {
    const discovered = await running.discoverProtocol(adapter.protocol);
    const options: HTMLOptionElement[] = [];
    const nextDiscoveredRoutes = new Map<string, string[]>();
    for (const candidate of discovered) {
      try {
        const routes = preferredBrowserRoutes(candidate.routes);
        const route = routes[0];
        const option = document.createElement("option");
        option.value = candidate.peerId;
        option.textContent = `${candidate.peerId} — expires ${candidate.expiresAt}`;
        option.dataset.route = route ?? "";
        option.dataset.source = candidate.source;
        option.disabled = !route;
        options.push(option);
        if (routes.length) nextDiscoveredRoutes.set(candidate.peerId, routes);
      } finally { candidate.free(); }
    }
    if (peer !== running) return;
    discoveredRoutes = nextDiscoveredRoutes;
    if (options.length) {
      candidates.replaceChildren(...options);
      candidates.disabled = false;
      useCandidateButton.disabled = !selectedCandidate();
      write(`discovered ${options.length} Echo peer(s)`);
    } else {
      const empty = document.createElement("option");
      empty.value = "";
      empty.textContent = "No discovered Echo peers";
      candidates.replaceChildren(empty);
      candidates.disabled = useCandidateButton.disabled = true;
      write("discovered 0 Echo peers");
    }
  } catch (error) {
    write(error);
  } finally {
    if (peer === running) refreshButton.disabled = false;
  }
}

function preferredBrowserRoutes(routes: string[]): string[] {
  return routes
    .filter((route) => route.split("/").includes("wss"))
    .sort((left, right) => {
      const leftRelay = left.includes("/p2p-circuit/");
      const rightRelay = right.includes("/p2p-circuit/");
      return leftRelay === rightRelay ? left.localeCompare(right) : leftRelay ? -1 : 1;
    });
}

function selectedCandidate(): HTMLOptionElement | undefined {
  const option = candidates.selectedOptions.item(0);
  return option instanceof HTMLOptionElement && option.value && option.dataset.route
    ? option
    : undefined;
}

function useSelectedCandidate(): void {
  const option = selectedCandidate();
  if (!option) return;
  input("remote-peer").value = option.value;
  input("remote-route").value = option.dataset.route!;
  write(`selected discovered peer ${option.value}; exact send will authenticate it`);
}

async function sendEcho(): Promise<void> {
  const adapter = echo;
  if (!adapter) return;
  sendButton.disabled = stopButton.disabled = true;
  try {
    const peerId = input("remote-peer").value.trim();
    const route = input("remote-route").value.trim();
    const advertised = discoveredRoutes.get(peerId);
    const routes = advertised?.[0] === route ? advertised : [route];
    const failures: string[] = [];
    for (const candidateRoute of routes) {
      try {
        const receipt = await adapter.sendExact(
          peerId, candidateRoute, new TextEncoder().encode(input("message").value),
        );
        try {
          write(`sent to ${receipt.remotePeerId}: ${new TextDecoder().decode(receipt.payload)}`);
        } finally { receipt.free(); }
        return;
      } catch (error) {
        failures.push(`${candidateRoute}: ${String(error)}`);
      }
    }
    throw new Error(`every candidate route failed: ${failures.join("; ")}`);
  } catch (error) {
    write(error);
  } finally {
    if (echo === adapter) sendButton.disabled = stopButton.disabled = false;
  }
}

async function receiveEchoes(mounted: AukiEcho): Promise<void> {
  while (echoEndpoint === mounted) try {
    const receipt = await mounted.nextServed();
    try { write(`received from ${receipt.remotePeerId}: ${new TextDecoder().decode(receipt.payload)}`); }
    finally { receipt.free(); }
  } catch (error) {
    if (echoEndpoint === mounted) write(error);
    return;
  }
}

async function stopPeer(): Promise<void> {
  const running = peer;
  const adapter = echo;
  const mounted = echoEndpoint;
  const pendingRefresh = refreshing;
  const pending = [sending, receiving];
  peer = echo = echoEndpoint = undefined;
  refreshing = receiving = undefined;
  if (!running || !adapter) return;
  sendButton.disabled = stopButton.disabled = true;
  let failure: unknown;
  if (pendingRefresh) await Promise.allSettled([pendingRefresh]);
  if (mounted) try { await mounted.close(); } catch (error) { failure = error; }
  try { await running.shutdown(); } catch (error) { failure ??= error; }
  await Promise.allSettled(pending);
  adapter.free();
  running.free();
  local.textContent = "Not connected";
  for (const key of Object.keys(local.dataset)) delete local.dataset[key];
  discovery.hidden = true;
  refreshButton.disabled = useCandidateButton.disabled = true;
  candidates.disabled = true;
  candidates.replaceChildren(new Option("No discovered peers", ""));
  discoveredRoutes = new Map();
  input("remote-peer").value = input("remote-route").value = "";
  input("email").disabled = input("password").disabled = false;
  get<HTMLButtonElement>("login-button").disabled = false;
  domain.replaceChildren();
  write(failure ?? "Peer stopped");
}

function selectedDiscoveryMode(): AukiDiscoveryMode {
  switch (discoveryMode.value) {
    case "discover_only": return AukiDiscoveryMode.DiscoverOnly;
    case "discover_and_advertise": return AukiDiscoveryMode.DiscoverAndAdvertise;
    default: throw new Error("Select an explicit DDS discovery mode");
  }
}

function selectedReachabilityMode(): AukiPeerReachabilityMode {
  switch (reachabilityMode.value) {
    case "outbound_only": return AukiPeerReachabilityMode.OutboundOnly;
    case "relay_backed": return AukiPeerReachabilityMode.RelayBacked;
    default: throw new Error("Select an explicit peer reachability mode");
  }
}

function syncReachabilityControls(): void {
  const advertise = discoveryMode.querySelector<HTMLOptionElement>(
    'option[value="discover_and_advertise"]',
  )!;
  const outboundOnly = reachabilityMode.value === "outbound_only";
  advertise.disabled = outboundOnly;
  if (outboundOnly) discoveryMode.value = "discover_only";
}

function write(value: unknown): void {
  const line = value instanceof Error ? value.message : String(value);
  log.textContent = `${log.textContent}${line}\n`.slice(-8_192);
}
