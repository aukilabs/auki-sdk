import init, { AukiEcho, AukiPeer, AukiUserSession } from "../pkg-web/auki_portable_echo_web.js";
const get = <T extends HTMLElement>(id: string): T => document.querySelector<T>(`#${id}`)!;
const input = (id: string): HTMLInputElement => get<HTMLInputElement>(id);
const startButton = get<HTMLButtonElement>("start-button");
const sendButton = get<HTMLButtonElement>("send-button");
const stopButton = get<HTMLButtonElement>("stop-button");
const local = get<HTMLElement>("local");
const log = get<HTMLElement>("log");
const MAX_LOG_CHARACTERS = 8_192;
let peer: AukiPeer | undefined;
let echo: AukiEcho | undefined;
let receiving: Promise<void> | undefined;
let sending: Promise<void> | undefined;
await init();
write("Runtime ready");
get<HTMLFormElement>("start").onsubmit = (event) => (event.preventDefault(), void startPeer());
get<HTMLFormElement>("send").onsubmit = (event) => {
  event.preventDefault();
  if (!sending) sending = sendEcho().finally(() => { sending = undefined; });
};
const stop = (): void => { void stopPeer().catch(write); };
stopButton.onclick = stop;
async function startPeer(): Promise<void> {
  let session: AukiUserSession | undefined;
  startButton.disabled = true;
  try {
    session = await AukiUserSession.loginDev(input("email").value, input("password").value);
    const started = await session.startPeer(input("domain").value.trim());
    try {
      echo = new AukiEcho(started);
      peer = started;
    } catch (error) {
      try { await started.shutdown(); } finally { started.free(); }
      throw error;
    }
    local.textContent = `Peer ID: ${started.peerId}\nWSS route: ${started.wssRoute}`;
    sendButton.disabled = stopButton.disabled = false;
    receiving = receiveEchoes(echo);
    void started.waitStopped().catch((error) => peer === started && (write(error), stop()));
  } catch (error) {
    startButton.disabled = false;
    write(error);
  } finally {
    input("password").value = "";
    session?.free();
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
    write(`sent: ${new TextDecoder().decode(receipt.payload)}`);
    receipt.free();
  } catch (error) {
    write(error);
  } finally {
    if (echo === mounted) sendButton.disabled = stopButton.disabled = false;
  }
}
async function receiveEchoes(mounted: AukiEcho): Promise<void> {
  while (echo === mounted) try {
    const receipt = await mounted.nextServed();
    write(`received from ${receipt.remotePeerId}: ${new TextDecoder().decode(receipt.payload)}`);
    receipt.free();
  } catch (error) {
    if (echo === mounted) write(error);
    return;
  }
}
async function stopPeer(): Promise<void> {
  const running = peer;
  const mounted = echo;
  const pendingReceive = receiving;
  const pendingSend = sending;
  peer = echo = undefined;
  receiving = undefined;
  if (!running || !mounted) return;
  sendButton.disabled = stopButton.disabled = true;
  let failure: unknown;
  try { await mounted.close(); } catch (error) { failure = error; }
  try { await running.shutdown(); } catch (error) { failure ??= error; }
  await Promise.allSettled([pendingSend, pendingReceive]);
  mounted.free();
  running.free();
  local.textContent = "Not connected";
  startButton.disabled = false;
  write(failure ?? "Peer stopped");
}
function write(value: unknown): void {
  const line = value instanceof Error ? value.message : String(value);
  log.textContent = `${log.textContent}${line}\n`.slice(-MAX_LOG_CHARACTERS);
}
