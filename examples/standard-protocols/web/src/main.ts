import init, { AukiUserSession } from "../pkg-web/auki_sdk_web.js";
import { BrowserPlayground, type PeerCard, type ProbeResult } from "./playground.js";

const get = <T extends HTMLElement>(id: string): T => document.querySelector<T>(`#${id}`)!;
const input = (id: string): HTMLInputElement => get<HTMLInputElement>(id);
const loginForm = get<HTMLFormElement>("login");
const startForm = get<HTMLFormElement>("start");
const probeForm = get<HTMLFormElement>("probe");
const domain = get<HTMLSelectElement>("domain");
const local = get<HTMLElement>("local");
const remoteCard = get<HTMLTextAreaElement>("remote-card");
const log = get<HTMLElement>("log");
let session: AukiUserSession | undefined;
let playground: BrowserPlayground | undefined;

await init();
get<HTMLButtonElement>("login-button").disabled = false;
write("Runtime ready");

loginForm.onsubmit = (event) => { event.preventDefault(); void login(); };
startForm.onsubmit = (event) => { event.preventDefault(); void startPeer(); };
probeForm.onsubmit = (event) => { event.preventDefault(); void probe(); };
get<HTMLButtonElement>("copy").onclick = () => void navigator.clipboard.writeText(local.textContent ?? "");
get<HTMLButtonElement>("stop-button").onclick = () => void stopPeer().catch(write);

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
    playground = await BrowserPlayground.start(authenticated, domain.value, "browser-playground", write);
    session = undefined;
    authenticated.free();
    showCard(playground.card());
    startForm.hidden = true;
    get<HTMLButtonElement>("probe-button").disabled = false;
    get<HTMLButtonElement>("stop-button").disabled = false;
    write("All six standard protocols are serving");
  } catch (error) {
    get<HTMLButtonElement>("start-button").disabled = false;
    write(error);
  }
}

async function probe(): Promise<void> {
  const running = playground;
  if (!running) return;
  const button = get<HTMLButtonElement>("probe-button");
  button.disabled = true;
  try {
    const target = JSON.parse(remoteCard.value) as PeerCard;
    const result = await running.probeAll(target);
    write(result.ok ? `All six protocols passed against ${target.peerId}` : JSON.stringify(result.errors));
  } catch (error) {
    write(error);
  } finally {
    if (playground === running) button.disabled = false;
  }
}

async function stopPeer(): Promise<void> {
  const running = playground;
  playground = undefined;
  if (!running) return;
  get<HTMLButtonElement>("probe-button").disabled = true;
  get<HTMLButtonElement>("stop-button").disabled = true;
  await running.close();
  local.textContent = "Peer stopped";
  for (const key of Object.keys(local.dataset)) delete local.dataset[key];
  input("email").disabled = false;
  input("password").disabled = false;
  get<HTMLButtonElement>("login-button").disabled = false;
  write("Peer stopped");
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
  start(input: { email: string; password: string; domainId: string; label: string }): Promise<PeerCard>;
  card(): PeerCard;
  probeAll(target: PeerCard): Promise<ProbeResult>;
  stop(): Promise<void>;
}

declare global { interface Window { aukiE2e: E2eApi; } }

window.aukiE2e = {
  async start(input): Promise<PeerCard> {
    if (playground) await stopPeer();
    const authenticated = await AukiUserSession.loginDev(input.email, input.password);
    try {
      playground = await BrowserPlayground.start(authenticated, input.domainId, input.label, write);
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
  probeAll(target): Promise<ProbeResult> {
    if (!playground) throw new Error("browser playground is not running");
    return playground.probeAll(target);
  },
  stop: stopPeer,
};
