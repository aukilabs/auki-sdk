import init, {
  AukiDiscoveryMode,
  AukiMapping,
  AukiPeer,
  AukiPeerReachabilityMode,
  AukiUserSession,
} from "../pkg-web/auki_collaborative_mapping_web.js";
import { Grid, type View, type Portal } from "./grid";
import { MapViews } from "./map-views";
import { DomainDiscovery } from "./discovery";
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
  discoveryTask: Promise<void> | undefined,
  stopping: Promise<void> | undefined;
let discovery: DomainDiscovery | undefined;
let busy = false;
const dialog = get<HTMLDialogElement>("pin-dialog");
let pinFrame = "",
  originalName: string | undefined;
const board = new Grid(
  get<SVGSVGElement>("grid"),
  dropPin,
  inspectPin,
  beginMove,
);
const maps = new MapViews(board, inspectPin);
board.render([]);

function render(json: string): void {
  latest = JSON.parse(json) as View;
  maps.render(latest);
}
function beginMove(
  portal: Portal,
): ((x: number, y: number) => void) | undefined {
  if (
    !mapping ||
    !latest ||
    busy ||
    stopping ||
    dialog.open ||
    !maps.editable ||
    !portal.contributors.includes(latest.local_peer)
  )
    return;
  const own = latest.layers
    .find((l) => l.peer === latest!.local_peer)
    ?.portals.find((p) => p.id === portal.id);
  if (!own) return;
  const current = mapping,
    frame = maps.placementFrame;
  return (x, y) => {
    if (mapping !== current || stopping || maps.placementFrame !== frame)
      return;
    const now = latest?.layers
      .find((l) => l.peer === latest!.local_peer)
      ?.portals.find((p) => p.id === own.id);
    if (!now || now.x !== own.x || now.y !== own.y) {
      notice("Portal changed while dragging. Try again.");
      return;
    }
    try {
      render(current.place(own.name, x, y, frame));
      notice(`Moved ${own.name}.`);
    } catch (error) {
      render(current.view());
      notice(error);
    }
  };
}
function dropPin(x: number, y: number): void {
  if (!mapping || busy || stopping || !maps.editable || dialog.open) return;
  pinFrame = maps.placementFrame;
  originalName = undefined;
  input("portal-name").value = "";
  input("portal-name").readOnly = false;
  input("x").value = String(x);
  input("y").value = String(y);
  input("x").readOnly = input("y").readOnly = false;
  button("save-pin").hidden = false;
  button("remove-pin").hidden = true;
  get("pin-title").textContent = "Drop a portal";
  get("pin-context").textContent =
    "Give this place a name. Matching names connect maps.";
  get("pin-error").textContent = "";
  board.setDraft({ x, y });
  dialog.showModal();
  input("portal-name").focus();
}
function inspectPin(portal: Portal): void {
  if (!mapping || !latest || stopping || dialog.open) return;
  const own = latest.layers
    .find((l) => l.peer === latest!.local_peer)!
    .portals.find((p) => p.id === portal.id);
  originalName = own?.name;
  const point = own ?? portal;
  pinFrame = own
    ? latest.layers.find((l) => l.peer === latest!.local_peer)!.frame
    : maps.placementFrame;
  input("portal-name").value = point.name;
  input("portal-name").readOnly = true;
  input("x").value = String(point.x);
  input("y").value = String(point.y);
  input("x").readOnly = input("y").readOnly = !own;
  button("save-pin").hidden = !own;
  button("remove-pin").hidden = !own;
  get("pin-title").textContent = own ? "Edit your portal" : "Peer portal";
  get("pin-context").textContent = own
    ? "Coordinates in your original frame. Changes update your map only."
    : "Read only. This placement belongs to another peer.";
  get("pin-error").textContent = "";
  dialog.showModal();
}
function closePin(): void {
  dialog.close();
  board.setDraft();
  get<SVGSVGElement>("grid").focus();
}
button("cancel-pin").onclick = closePin;
dialog.addEventListener("close", () => board.setDraft());
get<HTMLFormElement>("place").onsubmit = (event) => {
  event.preventDefault();
  if (!mapping || stopping) return;
  try {
    render(
      mapping.place(
        input("portal-name").value,
        Number(input("x").value),
        Number(input("y").value),
        pinFrame,
      ),
    );
    closePin();
    notice("Portal saved to your map.");
  } catch (error) {
    get("pin-error").textContent =
      error instanceof Error ? error.message : String(error);
  }
};
button("remove-pin").onclick = () => {
  if (!mapping || !originalName || stopping) return;
  try {
    render(mapping.remove(originalName));
    closePin();
    notice("Removed your placement. Peer maps are unchanged.");
  } catch (error) {
    get("pin-error").textContent = String(error);
  }
};
button("add-pin").onclick = () => board.dropAtCenter();
button("fit").onclick = () => board.fit();
button("zoom-in").onclick = () => board.zoom(0.8);
button("zoom-out").onclick = () => board.zoom(1.25);
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
    started = await session.startPeerWithDiscovery(
      get<HTMLSelectElement>("domain").value,
      AukiDiscoveryMode.DiscoverAndAdvertise,
      AukiPeerReachabilityMode.RelayBacked,
    );
    const mounted = await AukiMapping.mount(started, input("session").value);
    peer = started;
    started = undefined;
    mapping = mounted;
    get("start").hidden = true;
    get("running").hidden = false;
    get("peer-id").textContent = peer.peerId;
    get("session-name").textContent = input("session").value;
    render(mapping.view());
    get("transport").textContent = "Relay ready · discovering matching peers";
    notice(
      "Peers in the same Domain and demo session connect automatically in both directions.",
    );
    startDiscovery();
    const running = peer;
    void running.waitStopped().then(
      () => {
        if (peer === running && !stopping) {
          notice(
            "Peer stopped. Start again to advertise and discover this session.",
          );
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
function startDiscovery(): void {
  if (!peer || !mapping || discoveryTask || stopping) return;
  const running = peer,
    current = mapping;

  const controller = new DomainDiscovery({
    localPeer: running.peerId,
    async discover() {
      const candidates = await running.discoverProtocol(current.protocol);
      return candidates.map((candidate) => {
        try {
          return {
            peerId: candidate.peerId,
            routes: candidate.routes,
            expiresAt: candidate.expiresAt,
          };
        } finally {
          candidate.free();
        }
      });
    },
    inspect: (peerId, route) => current.inspectPeer(peerId, route),
    async follow(selection, peerId) {
      try {
        await current.follow(
          selection,
          (json: string) => {
            if (mapping === current && !stopping) render(json);
          },
          (status: string) => {
            if (mapping === current && !stopping) {
              maps.statuses.set(peerId, status);
              render(current.view());
            }
          },
        );
      } finally {
        maps.statuses.delete(peerId);
        if (mapping === current && !stopping) render(current.view());
      }
    },
    unfollow: async (peerId) => {
      await current.unfollow(peerId);
    },
    status(message) {
      if (mapping === current && !stopping)
        get("transport").textContent = message;
    },
  });
  discovery = controller;
  discoveryTask = controller
    .run()
    .catch((error) => {
      if (mapping === current && !stopping) {
        get("transport").textContent = "Session discovery stopped";
        notice(error);
      }
    })
    .finally(() => {
      if (discovery === controller) {
        discovery = undefined;
        discoveryTask = undefined;
      }
    });
}
button("stop-button").onclick = () => {
  void stop().catch(notice);
};
function stop(): Promise<void> {
  if (stopping) return stopping;
  stopping = (async () => {
    const current = mapping,
      running = peer;
    const pendingDiscovery = discoveryTask;
    discovery?.cancel();
    mapping = undefined;
    peer = undefined;
    closePin();
    button("add-pin").disabled = button("stop-button").disabled = true;
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
    if (pendingDiscovery) await pendingDiscovery;
    current?.free();
    running?.free();
    latest = undefined;
    maps.clear();
    get("running").hidden = true;
    get("start").hidden = !session;
    get("transport").textContent = "Not connected";
    button("stop-button").disabled = false;
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
