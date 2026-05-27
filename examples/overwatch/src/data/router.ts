// Hash-based router.
//   `#/`                     → directory
//   `#/robot/<encoded-url>`  → robot detail (live)
//   `#/cluster`              → ansuz participant list (libp2p peers)
// No external router library — Park's nav surface is small and a hash
// router gives us back/forward + deep-linking with zero deps and no
// server-side routing changes (rust-embed already SPA-falls back to
// index.html for unknown paths).

export type Route =
  | { view: "directory" }
  | { view: "robot"; url: string }
  | { view: "cluster" };

type Listener = (route: Route) => void;

const listeners = new Set<Listener>();
let installed = false;

export function getRoute(): Route {
  return parse(window.location.hash);
}

export function navigate(route: Route) {
  const next = serialize(route);
  if (window.location.hash !== next) {
    window.location.hash = next;
  } else {
    // Hash didn't change but we still want to re-emit (e.g. a manual call
    // to the same view). Trigger by dispatching the same event.
    notify(route);
  }
}

export function onRouteChange(cb: Listener): () => void {
  listeners.add(cb);
  ensureInstalled();
  cb(getRoute());
  return () => listeners.delete(cb);
}

function ensureInstalled() {
  if (installed) return;
  installed = true;
  window.addEventListener("hashchange", () => notify(getRoute()));
}

function notify(route: Route) {
  listeners.forEach((cb) => cb(route));
}

function parse(hash: string): Route {
  // Tolerate "", "#", "#/", "#/anything"
  const stripped = hash.replace(/^#\/?/, "");
  if (stripped === "") return { view: "directory" };
  if (stripped === "cluster") return { view: "cluster" };
  const robotMatch = /^robot\/(.+)$/.exec(stripped);
  if (robotMatch && robotMatch[1]) {
    return { view: "robot", url: decodeURIComponent(robotMatch[1]) };
  }
  // Unknown route — fall back to directory.
  return { view: "directory" };
}

function serialize(route: Route): string {
  switch (route.view) {
    case "directory":
      return "#/";
    case "robot":
      return `#/robot/${encodeURIComponent(route.url)}`;
    case "cluster":
      return "#/cluster";
  }
}
