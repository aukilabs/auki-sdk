import { sdkRuntime } from "./runtime";
import type { SensorSummary } from "./contract";

export type RuntimeStreamDescriptor = {
  sensor_id: string;
  sensor_hash: string;
  clock_id: string;
  clock_hash: string;
  frame_id: string;
  frame_hash: string;
};

export type RuntimeStreamFrame = {
  descriptor: RuntimeStreamDescriptor | null;
  payload: Uint8Array;
  sensorKind?: SensorSummary["kind"];
  seq: number;
  timestamp_ns: number;
  receivedAt: number;
  receivedAtWallMs: number;
};

type StreamSpec = {
  peer_id: string;
  sensor_id: string;
};

type Listener = (frame: RuntimeStreamFrame | null) => void;

type Source = {
  spec: StreamSpec;
  listeners: Set<Listener>;
  current: RuntimeStreamFrame | null;
  descriptor: RuntimeStreamDescriptor | null;
  stream: { close?: () => void | Promise<void> } | null;
  cancelled: boolean;
  state: "connecting" | "live" | "declined" | "rejected";
};

const sources = new Map<string, Source>();

export function subscribeRuntimeStream(spec: StreamSpec, cb: Listener): () => void {
  const key = cacheKey(spec);
  let source = sources.get(key);
  if (!source) {
    source = {
      spec,
      listeners: new Set(),
      current: null,
      descriptor: null,
      stream: null,
      cancelled: false,
      state: "connecting",
    };
    sources.set(key, source);
    void pump(source);
  }
  source.listeners.add(cb);
  cb(source.current);
  return () => {
    source?.listeners.delete(cb);
    if (source && source.listeners.size === 0) {
      source.cancelled = true;
      void source.stream?.close?.();
      sources.delete(key);
    }
  };
}

export function getRuntimeStreamDescriptor(spec: StreamSpec): RuntimeStreamDescriptor | null {
  return sources.get(cacheKey(spec))?.descriptor ?? null;
}

export function getRuntimeStreamState(spec: StreamSpec): Source["state"] | "unknown" {
  return sources.get(cacheKey(spec))?.state ?? "unknown";
}

async function pump(source: Source) {
  try {
    const sensor = sdkRuntime
      .getParticipantSensors(source.spec.peer_id)
      .find((candidate) => candidate.sensor_id === source.spec.sensor_id);
    const stream = await sdkRuntime.getStream(source.spec.peer_id, source.spec.sensor_id);
    source.stream = stream;
    while (!source.cancelled) {
      const message = await stream.nextMessage();
      if (message == null) break;
      if (isAcceptMessage(message)) {
        source.descriptor = message.accept;
        source.state = "live";
        continue;
      }
      if (isEntryMessage(message)) {
        const entry = message.entry;
        const frame: RuntimeStreamFrame = {
          descriptor: source.descriptor,
          payload: toBytes(entry.payload),
          sensorKind: sensor?.kind,
          seq: Number(entry.seq ?? 0),
          timestamp_ns: Number(entry.timestamp_ns ?? 0),
          receivedAt: performance.now(),
          receivedAtWallMs: Date.now(),
        };
        source.current = frame;
        source.listeners.forEach((cb) => cb(frame));
        continue;
      }
      if (isDeclineMessage(message)) {
        source.state = "declined";
        break;
      }
    }
  } catch {
    source.state = source.descriptor ? "rejected" : "rejected";
  }
}

function cacheKey(spec: StreamSpec): string {
  return `${spec.peer_id}::${spec.sensor_id}`;
}

function isAcceptMessage(message: unknown): message is { accept: RuntimeStreamDescriptor } {
  return Boolean(message && typeof message === "object" && "accept" in message);
}

function isEntryMessage(message: unknown): message is {
  entry: { payload: number[] | Uint8Array; seq?: number; timestamp_ns?: number };
} {
  return Boolean(message && typeof message === "object" && "entry" in message);
}

function isDeclineMessage(message: unknown): message is { decline: unknown } {
  return Boolean(message && typeof message === "object" && "decline" in message);
}

function toBytes(payload: number[] | Uint8Array): Uint8Array {
  if (payload instanceof Uint8Array) return payload;
  return Uint8Array.from(payload);
}
