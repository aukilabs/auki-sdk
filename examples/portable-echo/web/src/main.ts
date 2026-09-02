import init, {
  AukiDiscoveryMode,
  AukiEcho,
  AukiPeer,
  AukiUserSession,
} from "../pkg-web/auki_portable_echo_web.js";

const get = <T extends HTMLElement>(id: string): T => document.querySelector<T>(`#${id}`)!;
const input = (id: string): HTMLInputElement => get<HTMLInputElement>(id);
const login = get<HTMLFormElement>("login");
const start = get<HTMLFormElement>("start");
const domain = get<HTMLSelectElement>("domain");
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
let echo: AukiEcho | undefined;
let receiving: Promise<void> | undefined;
let sending: Promise<void> | undefined;
let refreshing: Promise<void> | undefined;

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
    );
    let mounted: AukiEcho;
    try { mounted = new AukiEcho(started); }
    catch (error) { try { await started.shutdown(); } finally { started.free(); } throw error; }
    session = undefined;
    authenticated.free();
    peer = started;
    echo = mounted;
    local.dataset.peerId = started.peerId;
    local.dataset.domainId = started.domainId;
    local.dataset.wssRoute = started.wssRoute;
    local.dataset.tcpRoute = started.tcpRoute;
    local.textContent = `Peer ID: ${started.peerId}\nWSS route: ${started.wssRoute}`
      + `\nTCP route: ${started.tcpRoute}`;
    start.hidden = true;
    discovery.hidden = false;
    refreshButton.disabled = false;
    sendButton.disabled = stopButton.disabled = false;
    receiving = receiveEchoes(mounted);
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
  const mounted = echo;
  if (!running || !mounted) return;
  refreshButton.disabled = useCandidateButton.disabled = true;
  try {
    const discovered = await running.discoverProtocol(mounted.protocol);
    const options: HTMLOptionElement[] = [];
    for (const candidate of discovered) {
      try {
        const route = preferredBrowserRoute(candidate.routes);
        const option = document.createElement("option");
        option.value = candidate.peerId;
        option.textContent = `${candidate.peerId} — expires ${candidate.expiresAt}`;
        option.dataset.route = route ?? "";
        option.dataset.source = candidate.source;
        option.disabled = !route;
        options.push(option);
      } finally { candidate.free(); }
    }
    if (peer !== running) return;
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

function preferredBrowserRoute(routes: string[]): string | undefined {
  return routes.find((route) => route.includes("/wss/") && route.includes("/p2p-circuit/"))
    ?? routes.find((route) => route.includes("/wss/"));
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
  const mounted = echo;
  if (!mounted) return;
  sendButton.disabled = stopButton.disabled = true;
  try {
    const receipt = await mounted.sendExact(
      input("remote-peer").value.trim(), input("remote-route").value.trim(),
      new TextEncoder().encode(input("message").value),
    );
    try { write(`sent to ${receipt.remotePeerId}: ${new TextDecoder().decode(receipt.payload)}`); }
    finally { receipt.free(); }
  } catch (error) {
    write(error);
  } finally {
    if (echo === mounted) sendButton.disabled = stopButton.disabled = false;
  }
}

async function receiveEchoes(mounted: AukiEcho): Promise<void> {
  while (echo === mounted) try {
    const receipt = await mounted.nextServed();
    try { write(`received from ${receipt.remotePeerId}: ${new TextDecoder().decode(receipt.payload)}`); }
    finally { receipt.free(); }
  } catch (error) {
    if (echo === mounted) write(error);
    return;
  }
}

async function stopPeer(): Promise<void> {
  const running = peer;
  const mounted = echo;
  const pendingRefresh = refreshing;
  const pending = [sending, receiving];
  peer = echo = undefined;
  refreshing = receiving = undefined;
  if (!running || !mounted) return;
  sendButton.disabled = stopButton.disabled = true;
  let failure: unknown;
  if (pendingRefresh) await Promise.allSettled([pendingRefresh]);
  try { await mounted.close(); } catch (error) { failure = error; }
  try { await running.shutdown(); } catch (error) { failure ??= error; }
  await Promise.allSettled(pending);
  mounted.free();
  running.free();
  local.textContent = "Not connected";
  for (const key of Object.keys(local.dataset)) delete local.dataset[key];
  discovery.hidden = true;
  refreshButton.disabled = useCandidateButton.disabled = true;
  candidates.disabled = true;
  candidates.replaceChildren(new Option("No discovered peers", ""));
  input("remote-peer").value = input("remote-route").value = "";
  input("email").disabled = input("password").disabled = false;
  get<HTMLButtonElement>("login-button").disabled = false;
  domain.replaceChildren();
  write(failure ?? "Peer stopped");
}

function selectedDiscoveryMode(): AukiDiscoveryMode {
  switch (input("discovery-mode").value) {
    case "discover_only": return AukiDiscoveryMode.DiscoverOnly;
    case "discover_and_advertise": return AukiDiscoveryMode.DiscoverAndAdvertise;
    default: throw new Error("Select an explicit DDS discovery mode");
  }
}

function write(value: unknown): void {
  const line = value instanceof Error ? value.message : String(value);
  log.textContent = `${log.textContent}${line}\n`.slice(-8_192);
}
