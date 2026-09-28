import init, {
  AukiMapping,
  AukiPeer,
  AukiPeerReachabilityMode,
  AukiUserSession,
} from "../pkg-web/auki_collaborative_mapping_web.js";
import { Grid, type View } from "./grid";
import "./styles.css";

const get = <T extends Element = HTMLElement>(id: string): T =>
  document.getElementById(id) as unknown as T;
const input = (id: string) => get<HTMLInputElement>(id);
const button = (id: string) => get<HTMLButtonElement>(id);
const notice = (value: unknown) => {
  get("notice").textContent =
    value instanceof Error ? value.message : String(value);
};
let session: AukiUserSession | undefined,
  peer: AukiPeer | undefined,
  mapping: AukiMapping | undefined;
let latest: View | undefined,
  following: Promise<void> | undefined,
  stopping: Promise<void> | undefined;
let busy = false;
const board = new Grid(get<SVGSVGElement>("grid"), (x, y) => {
  if (!mapping || busy || stopping) return;
  input("x").value = String(x);
  input("y").value = String(y);
  if (input("portal-name").reportValidity()) place();
});
const partnerBoard = new Grid(get<SVGSVGElement>("remote-grid"));
board.render([], []);

function render(json: string): void {
  latest = JSON.parse(json) as View;
  const view = latest;
  const peers = [
    view.local_peer,
    ...(view.remote_peer ? [view.remote_peer] : []),
  ];
  board.render(view.portals, peers);
  partnerBoard.render(view.remote_portals, peers);
  get("empty").hidden = view.portals.length > 0;
  get("remote-panel").hidden = !view.remote_peer || view.state === "aligned";
  get("map-title").textContent =
    view.state === "aligned" ? "Our common ground" : "Your local map";
  get("alignment").textContent =
    (
      {
        waiting: "Waiting for partner",
        separate: "Not aligned yet",
        aligned: "Maps aligned",
        conflict: "Alignment conflict",
        pending: "Checking alignment",
      } as Record<string, string>
    )[view.state] ?? view.state;
  get("map-caption").textContent =
    view.state === "aligned"
      ? "Shared view · original maps preserved"
      : "Independent local frame";
  get("frame-caption").textContent = `Frame ${view.display_frame.slice(-8)}`;
  get("frame-caption").title = view.display_frame;
  get("portal-count").textContent =
    `${view.portals.length} portal${view.portals.length === 1 ? "" : "s"}`;
  get("sequences").textContent =
    `Local ${view.local_reference.sequence} · Partner ${view.remote_reference?.sequence ?? "—"}`;
  get("evidence").textContent =
    view.reason ??
    (view.shared_names.length
      ? view.shared_names.join(", ")
      : "No shared portals yet");
  if (view.reason) notice(view.reason);
}
function place(): void {
  if (!mapping || !latest || stopping) return;
  try {
    render(
      mapping.place(
        input("portal-name").value,
        Number(input("x").value),
        Number(input("y").value),
        latest.display_frame,
      ),
    );
    notice("Portal saved to your map.");
  } catch (error) {
    notice(error);
  }
}
get<HTMLFormElement>("place").onsubmit = (event) => {
  event.preventDefault();
  place();
};
button("fit").onclick = () => {
  if (latest)
    board.render(latest.portals, [
      latest.local_peer,
      ...(latest.remote_peer ? [latest.remote_peer] : []),
    ]);
};
get<HTMLSelectElement>("environment").onchange = () => {
  const custom = get<HTMLSelectElement>("environment").value === "custom";
  get("custom-environment").hidden = !custom;
  ["api", "dds", "dms"].forEach((id) => {
    input(id).required = custom;
  });
};
get<HTMLFormElement>("login").onsubmit = (event) => {
  event.preventDefault();
  void login();
};
async function login(): Promise<void> {
  if (busy || session) return;
  busy = true;
  button("login-button").disabled = true;
  let authenticated: AukiUserSession | undefined;
  try {
    authenticated =
      get<HTMLSelectElement>("environment").value === "custom"
        ? await AukiUserSession.loginWithEnvironment(
            input("api").value,
            input("dds").value,
            input("dms").value,
            input("email").value,
            input("password").value,
          )
        : await AukiUserSession.loginDev(
            input("email").value,
            input("password").value,
          );
    const domains = await authenticated.accessibleDomains();
    const options = domains.map((domain) => {
      try {
        return new Option(domain.name || domain.id, domain.id);
      } finally {
        domain.free();
      }
    });
    if (!options.length)
      throw new Error("No accessible Domains for this account.");
    get<HTMLSelectElement>("domain").replaceChildren(...options);
    session = authenticated;
    authenticated = undefined;
    get("login").hidden = true;
    get("start").hidden = false;
    button("logout-button").hidden = false;
    notice("Choose the same Domain and demo session in both browsers.");
  } catch (error) {
    notice(error);
  } finally {
    if (authenticated) {
      try {
        await authenticated.close();
      } catch (error) {
        notice(error);
      } finally {
        authenticated.free();
      }
    }
    input("password").value = "";
    busy = false;
    button("login-button").disabled = false;
  }
}
get<HTMLFormElement>("start").onsubmit = (event) => {
  event.preventDefault();
  void start();
};
async function start(): Promise<void> {
  if (!session || peer || busy || stopping) return;
  busy = true;
  button("start-button").disabled = button("logout-button").disabled = true;
  let started: AukiPeer | undefined;
  try {
    started = await session.startPeer(
      get<HTMLSelectElement>("domain").value,
      AukiPeerReachabilityMode.RelayBacked,
    );
    const mounted = await AukiMapping.mount(started, input("session").value);
    peer = started;
    started = undefined;
    mapping = mounted;
    get("start").hidden = true;
    get("running").hidden = false;
    get("peer-id").textContent = peer.peerId;
    get<HTMLTextAreaElement>("local-card").value = mapping.connectionCard();
    get<HTMLFieldSetElement>("editor").disabled = false;
    button("pair-button").disabled = false;
    render(mapping.view());
    get("transport").textContent = "Relay ready · exchange connection cards";
    notice(
      "Name a portal and click a grid cell. Pair both browsers to share maps.",
    );
    const running = peer;
    void running.waitStopped().then(
      () => {
        if (peer === running && !stopping) {
          notice("Peer stopped. Start again and exchange fresh cards.");
          void stop().catch(notice);
        }
      },
      (error) => {
        if (peer === running && !stopping) {
          notice(error);
          void stop().catch(notice);
        }
      },
    );
  } catch (error) {
    if (started) {
      try {
        await started.shutdown();
      } catch {
        /* Preserve startup error. */
      } finally {
        started.free();
      }
    }
    notice(error);
  } finally {
    busy = false;
    button("start-button").disabled = button("logout-button").disabled = false;
  }
}
button("copy-card").onclick = () => {
  void navigator.clipboard
    .writeText(get<HTMLTextAreaElement>("local-card").value)
    .then(
      () => notice("Connection card copied."),
      () => notice("Select and copy the card from the text box."),
    );
};
get<HTMLFormElement>("pair").onsubmit = (event) => {
  event.preventDefault();
  if (!mapping || following || stopping) return;
  const current = mapping;
  try {
    button("pair-button").disabled = true;
    const promise = mapping.follow(
      get<HTMLTextAreaElement>("remote-card").value,
      (json: string) => {
        if (mapping === current && !stopping) render(json);
      },
      (status: string) => {
        if (mapping === current && !stopping)
          get("transport").textContent = status;
      },
    );
    render(mapping.view());
    following = Promise.resolve(promise)
      .then(() => undefined)
      .catch((error) => {
        if (mapping === current && !stopping) {
          get("transport").textContent = "Disconnected · remote map is stale";
          notice(error);
        }
      })
      .finally(() => {
        following = undefined;
        if (!stopping) button("pair-button").disabled = false;
      });
  } catch (error) {
    notice(error);
    button("pair-button").disabled = false;
  }
};
button("stop-button").onclick = () => {
  void stop().catch(notice);
};
function stop(): Promise<void> {
  if (stopping) return stopping;
  stopping = (async () => {
    const current = mapping,
      running = peer;
    mapping = undefined;
    peer = undefined;
    get<HTMLFieldSetElement>("editor").disabled = true;
    button("stop-button").disabled = button("pair-button").disabled = true;
    let failure: unknown;
    try {
      if (current) await current.close();
    } catch (error) {
      failure = error;
    }
    try {
      if (running) await running.shutdown();
    } catch (error) {
      failure ??= error;
    }
    if (following) await following;
    current?.free();
    running?.free();
    latest = undefined;
    board.render([], []);
    get("running").hidden = get("remote-panel").hidden = true;
    get("start").hidden = !session;
    get("empty").hidden = false;
    get("map-title").textContent = "Your map starts here";
    get("alignment").textContent = "Peer stopped";
    get("transport").textContent = "Not connected";
    get("evidence").textContent = "No shared portals yet";
    get("sequences").textContent = "Local — · Partner —";
    get("portal-count").textContent = "0 portals";
    get("frame-caption").textContent = "No frame yet";
    get("map-caption").textContent = "Independent local frame";
    get<HTMLTextAreaElement>("local-card").value = get<HTMLTextAreaElement>(
      "remote-card",
    ).value = "";
    button("stop-button").disabled = button("pair-button").disabled = false;
    if (failure) throw failure;
    notice(
      "Peer stopped; relay cleanup completed. Start again for a fresh map.",
    );
  })().finally(() => {
    stopping = undefined;
  });
  return stopping;
}
button("logout-button").onclick = () => {
  void logout().catch(notice);
};
async function logout(): Promise<void> {
  if (busy) return;
  busy = true;
  button("logout-button").disabled = true;
  let failure: unknown;
  try {
    try {
      await stop();
    } catch (error) {
      failure = error;
    }
    const authenticated = session;
    session = undefined;
    if (authenticated) {
      try {
        await authenticated.close();
      } catch (error) {
        failure ??= error;
      } finally {
        authenticated.free();
      }
    }
    get("start").hidden = button("logout-button").hidden = true;
    get("login").hidden = false;
    if (failure) throw failure;
    notice("Signed out.");
  } finally {
    busy = false;
    button("logout-button").disabled = false;
  }
}
window.addEventListener("beforeunload", (event) => {
  if (peer || busy) {
    event.preventDefault();
    event.returnValue = "";
  }
});
try {
  await init();
  button("login-button").disabled = false;
  get("runtime").textContent = "Browser runtime ready";
} catch (error) {
  get("runtime").textContent = "WASM failed to load";
  notice(error);
}
