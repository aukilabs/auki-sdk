import {
  getRuntimeStreamState,
  subscribeRuntimeStream,
} from "../sdk/streamHub";
import { previewPayloadBytes } from "./cameraFramePayload";

export type StreamState =
  | "connecting"
  | "live"
  | "reconnecting"
  | "declined"
  | "rejected"
  | "unknown";

type Listener = (frame: PreviewFrame | null) => void;

export type PreviewFrame = {
  url: string;
  receivedAt: number;
  receivedAtWallMs: number;
  frameAgeMs: number;
  seq: number;
  sensorHash: string;
  clockId: string;
  bytes: number;
  timestamp_ns: number;
};

export type StreamSpec = {
  peer_id: string;
  sensor_id: string;
};

const URL_WINDOW = 24;

type Source = {
  spec: StreamSpec;
  listeners: Set<Listener>;
  current: PreviewFrame | null;
  recent: string[];
  unsubscribeRuntime: () => void;
};

function cacheKey(spec: StreamSpec): string {
  return `${spec.peer_id}::${spec.sensor_id}`;
}

const sources = new Map<string, Source>();

export function getStreamState(spec: StreamSpec): StreamState {
  return getRuntimeStreamState(spec);
}

export function subscribePreview(spec: StreamSpec, cb: Listener): () => void {
  const key = cacheKey(spec);
  let source = sources.get(key);
  if (!source) {
    source = {
      spec,
      listeners: new Set(),
      current: null,
      recent: [],
      unsubscribeRuntime: () => {},
    };
    source.unsubscribeRuntime = subscribeRuntimeStream(spec, (frame) => {
      if (!frame) return;
      const body = toArrayBuffer(previewPayloadBytes(frame.payload, frame.sensorKind));
      const blob = new Blob([body], { type: "image/jpeg" });
      const next: PreviewFrame = {
        url: URL.createObjectURL(blob),
        receivedAt: frame.receivedAt,
        receivedAtWallMs: frame.receivedAtWallMs,
        frameAgeMs: 0,
        seq: frame.seq,
        sensorHash: frame.descriptor?.sensor_hash ?? "",
        clockId: frame.descriptor?.clock_id ?? "",
        bytes: blob.size,
        timestamp_ns: frame.timestamp_ns,
      };
      source!.current = next;
      source!.recent.push(next.url);
      while (source!.recent.length > URL_WINDOW) {
        const old = source!.recent.shift();
        if (old) URL.revokeObjectURL(old);
      }
      source!.listeners.forEach((listener) => listener(next));
    });
    sources.set(key, source);
  }
  source.listeners.add(cb);
  cb(source.current);

  return () => {
    if (!source) return;
    source.listeners.delete(cb);
    if (source.listeners.size === 0) {
      source.unsubscribeRuntime();
      for (const url of source.recent) URL.revokeObjectURL(url);
      source.recent.length = 0;
      sources.delete(key);
    }
  };
}

function toArrayBuffer(bytes: Uint8Array): ArrayBuffer {
  const copy = new Uint8Array(bytes.byteLength);
  copy.set(bytes);
  return copy.buffer;
}
