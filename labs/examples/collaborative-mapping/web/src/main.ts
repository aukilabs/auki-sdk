import init, {
  AukiDiscoveryMode,
  AukiMapping,
  AukiPeer,
  AukiPeerReachabilityMode,
  AukiUserSession,
} from "../pkg-web/auki_collaborative_mapping_web.js";
import { Grid, type View } from "./grid";
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
    get<HTMLFieldSetElement>("editor").disabled = false;
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
  button("retry-discovery").disabled = true;
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
    async follow(selection) {
      get<HTMLSelectElement>("peer-choice").disabled = true;
      try {
        const task = current.follow(
          selection,
          (json: string) => {
            if (mapping === current && !stopping) render(json);
          },
          (status: string) => {
            if (mapping === current && !stopping)
              get("transport").textContent = status;
          },
        );
        render(current.view());
        await task;
      } finally {
        get<HTMLSelectElement>("peer-choice").disabled = false;
      }
    },
    matches(matches) {
      if (mapping !== current || stopping) return;
      get("partner-picker").hidden = matches.length <= 1;
      const select = get<HTMLSelectElement>("peer-choice"),
        previous = select.value;
      select.replaceChildren(
        new Option("Choose a partner", ""),
        ...matches.map((match) => new Option(match.peerId, match.peerId)),
      );
      if (matches.some((match) => match.peerId === previous))
        select.value = previous;
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
        get("transport").textContent = "Discovery paused · retry to reconnect";
        notice(error);
      }
    })
    .finally(() => {
      if (discovery === controller) {
        discovery = undefined;
        discoveryTask = undefined;
      }
      if (mapping === current && !stopping)
        button("retry-discovery").disabled = false;
    });
}
button("retry-discovery").onclick = startDiscovery;
get<HTMLSelectElement>("peer-choice").onchange = () =>
  discovery?.choose(get<HTMLSelectElement>("peer-choice").value);
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
    get<HTMLFieldSetElement>("editor").disabled = true;
    button("stop-button").disabled = button("retry-discovery").disabled = true;
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
    get("partner-picker").hidden = true;
    get<HTMLSelectElement>("peer-choice").replaceChildren();
    button("stop-button").disabled = button("retry-discovery").disabled = false;
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
