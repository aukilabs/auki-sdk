import init, { AukiEcho, AukiPeer, AukiUserSession } from "../pkg-web/auki_portable_echo_web.js";

const get = <T extends HTMLElement>(id: string): T => document.querySelector<T>(`#${id}`)!;
const input = (id: string): HTMLInputElement => get<HTMLInputElement>(id);
const login = get<HTMLFormElement>("login");
const start = get<HTMLFormElement>("start");
const domain = get<HTMLSelectElement>("domain");
const local = get<HTMLElement>("local");
const sendButton = get<HTMLButtonElement>("send-button");
const stopButton = get<HTMLButtonElement>("stop-button");
const log = get<HTMLElement>("log");
let session: AukiUserSession | undefined;
let peer: AukiPeer | undefined;
let echo: AukiEcho | undefined;
let receiving: Promise<void> | undefined;
let sending: Promise<void> | undefined;

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
    const started = await authenticated.startPeer(domain.value);
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
  const pending = [sending, receiving];
  peer = echo = undefined;
  receiving = undefined;
  if (!running || !mounted) return;
  sendButton.disabled = stopButton.disabled = true;
  let failure: unknown;
  try { await mounted.close(); } catch (error) { failure = error; }
  try { await running.shutdown(); } catch (error) { failure ??= error; }
  await Promise.allSettled(pending);
  mounted.free();
  running.free();
  local.textContent = "Not connected";
  for (const key of Object.keys(local.dataset)) delete local.dataset[key];
  input("email").disabled = input("password").disabled = false;
  get<HTMLButtonElement>("login-button").disabled = false;
  domain.replaceChildren();
  write(failure ?? "Peer stopped");
}

function write(value: unknown): void {
  const line = value instanceof Error ? value.message : String(value);
  log.textContent = `${log.textContent}${line}\n`.slice(-8_192);
}
