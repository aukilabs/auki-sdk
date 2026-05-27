// Per-daemon persisted toggle state. localStorage-backed so reopening
// a robot tomorrow restores the operator's last tile selection on
// that daemon. Keyed solely on the daemon URL — past sessions are
// gone, so there's nothing else to disambiguate by.

const KEY = "park.toggledSensors.v3";

type Map = Record<string, string[]>;

function load(): Map {
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw);
    if (parsed && typeof parsed === "object") return parsed as Map;
    return {};
  } catch {
    return {};
  }
}

function save(map: Map): void {
  try {
    localStorage.setItem(KEY, JSON.stringify(map));
  } catch {
    // localStorage full or disabled — just drop silently.
  }
}

export function getToggled(daemonUrl: string): Set<string> | null {
  const map = load();
  const ids = map[daemonUrl];
  if (!Array.isArray(ids)) return null;
  return new Set(ids);
}

export function setToggled(daemonUrl: string, toggled: Set<string>): void {
  const map = load();
  if (toggled.size === 0) {
    delete map[daemonUrl];
  } else {
    map[daemonUrl] = Array.from(toggled);
  }
  save(map);
}
