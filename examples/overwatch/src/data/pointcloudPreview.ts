import {
  decodePointCloud2,
  type DecodedPointCloud,
  type InferredPinholeProjection,
} from "./cdrPointCloud";
import { subscribeRuntimeStream } from "../sdk/streamHub";

type Listener = (frame: PointCloudPreviewFrame | null) => void;

export type PointCloudPreviewFrame = {
  frameId: string;
  height: number;
  width: number;
  positions: Float32Array;
  pointCount: number;
  projection?: InferredPinholeProjection;
  receivedAt: number;
  seq: number;
  timestamp_ns: number;
};

export type PointCloudSpec = {
  peer_id: string;
  sensor_id: string;
};

type Source = {
  spec: PointCloudSpec;
  listeners: Set<Listener>;
  current: PointCloudPreviewFrame | null;
  unsubscribeRuntime: () => void;
  lastSeenSeq: number | null;
};

function cacheKey(spec: PointCloudSpec): string {
  return `${spec.peer_id}::${spec.sensor_id}`;
}

const sources = new Map<string, Source>();

export function subscribePointCloud(
  spec: PointCloudSpec,
  cb: Listener,
): () => void {
  const key = cacheKey(spec);
  let source = sources.get(key);
  if (!source) {
    source = {
      spec,
      listeners: new Set(),
      current: null,
      unsubscribeRuntime: () => {},
      lastSeenSeq: null,
    };
    source.unsubscribeRuntime = subscribeRuntimeStream(spec, (frame) => {
      if (!frame) return;
      if (source!.lastSeenSeq === frame.seq) return;
      source!.lastSeenSeq = frame.seq;

      let decoded: DecodedPointCloud;
      try {
        decoded = decodePointCloud2(toArrayBuffer(frame.payload));
      } catch (err) {
        console.warn(`pointcloudPreview: decode failed for ${spec.sensor_id}:`, err);
        return;
      }

      const next: PointCloudPreviewFrame = {
        frameId: decoded.frameId,
        height: decoded.height,
        width: decoded.width,
        positions: decoded.positions,
        pointCount: decoded.pointCount,
        projection: decoded.projection,
        receivedAt: frame.receivedAt,
        seq: frame.seq,
        timestamp_ns: frame.timestamp_ns,
      };
      source!.current = next;
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
      sources.delete(key);
    }
  };
}

function toArrayBuffer(bytes: Uint8Array): ArrayBuffer {
  const copy = new Uint8Array(bytes.byteLength);
  copy.set(bytes);
  return copy.buffer;
}
