// Operator-selected dashboard preview sensor, persisted per peer.
//
// The dashboard card should not guess which of a robot's cameras is
// the "right" one. Store the operator's choice locally and fall back to
// stable catalog ordering until they choose.

const KEY = "park.dashboardPreviewSensor.v1";

function readMap(): Record<string, string> {
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw);
    return parsed && typeof parsed === "object" ? parsed : {};
  } catch {
    return {};
  }
}

function writeMap(map: Record<string, string>) {
  localStorage.setItem(KEY, JSON.stringify(map));
}

export function getDashboardPreviewSensor(peerId: string): string | null {
  return readMap()[peerId] ?? null;
}

export function setDashboardPreviewSensor(peerId: string, sensorId: string | null) {
  const map = readMap();
  if (sensorId) map[peerId] = sensorId;
  else delete map[peerId];
  writeMap(map);
}
