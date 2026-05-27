export type RecordingSensorKind =
  | "camera"
  | "point_cloud"
  | "joint_encoders"
  | "audio";

export type RecordingStatus =
  | "starting"
  | "recording"
  | "reconnecting"
  | "stopping"
  | "stopped"
  | "error";

export type RecordingWire = {
  id: string;
  peer_id: string;
  sensor_id: string;
  sensor_kind: RecordingSensorKind;
  status: RecordingStatus;
  started_at_unix_ns: number;
  stopped_at_unix_ns: number | null;
  log_path: string;
  frames_written: number;
  bytes_written: number;
  last_timestamp_ns: number | null;
  sensor_hash: string | null;
  clock_id: string | null;
  clock_hash: string | null;
  frame_id: string | null;
  frame_hash: string | null;
  render_path: string | null;
  render_error: string | null;
  last_error: string | null;
};

type Listener = (recordings: RecordingWire[]) => void;

const listeners = new Set<Listener>();
let current: RecordingWire[] = [];

export function subscribeRecordings(cb: Listener): () => void {
  listeners.add(cb);
  cb(current);
  return () => listeners.delete(cb);
}

export async function refreshRecordings(): Promise<RecordingWire[]> {
  return current;
}

export async function startIntentRecording(
  _peerId: string,
  _sensorId: string,
): Promise<RecordingWire> {
  throw new Error("Browser Overwatch is live-only; SDK recording control is not available in this example.");
}

export async function stopIntentRecording(_id: string): Promise<RecordingWire> {
  throw new Error("Browser Overwatch is live-only; SDK recording control is not available in this example.");
}

export function activeRecordingForSensor(
  recordings: RecordingWire[],
  peerId: string,
  sensorId: string,
): RecordingWire | null {
  const matches = recordings.filter(
    (r) => r.peer_id === peerId && r.sensor_id === sensorId,
  );
  return matches.find(isActiveRecording) ?? matches[0] ?? null;
}

export function isActiveRecording(r: RecordingWire): boolean {
  return (
    r.status === "starting" ||
    r.status === "recording" ||
    r.status === "reconnecting" ||
    r.status === "stopping"
  );
}
