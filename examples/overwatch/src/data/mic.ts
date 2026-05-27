export type MicSnapshot = {
  enabled: boolean;
  sensorId: string;
  refreshedAtMs: number;
};

type Listener = (snap: MicSnapshot) => void;

const listeners = new Set<Listener>();
let current: MicSnapshot = {
  enabled: false,
  sensorId: "",
  refreshedAtMs: Date.now(),
};

export function subscribeMic(cb: Listener): () => void {
  listeners.add(cb);
  cb(current);
  return () => listeners.delete(cb);
}

export function getMic(): MicSnapshot {
  return current;
}

export async function setMic(
  enabled: boolean,
  _peerId?: string | null,
): Promise<MicSnapshot> {
  if (enabled) {
    throw new Error("Browser Overwatch audio is not wired in this pass.");
  }
  current = { enabled: false, sensorId: "", refreshedAtMs: Date.now() };
  listeners.forEach((cb) => cb(current));
  return current;
}
