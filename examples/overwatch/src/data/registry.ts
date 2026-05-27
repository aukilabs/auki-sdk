import initGeometry, { conventionMatrixJson } from "@aukilabs/auki-geometry";

import { sdkRuntime } from "../sdk/runtime";
import { getRuntimeStreamDescriptor } from "../sdk/streamHub";

export type SensorEntry =
  | (SensorEntryBase & CameraBody)
  | (SensorEntryBase & PointCloudBody)
  | (SensorEntryBase & AudioBody)
  | (SensorEntryBase & JointEncodersBody);

export type CameraEntry = Extract<SensorEntry, { type: "camera" }>;

type SensorEntryBase = {
  sensor_id: string;
};

export type CameraBody = {
  type: "camera";
  width: number;
  height: number;
  frame_rate_hz: number;
  pixel_format: string;
  color_space: string;
  intrinsics_model: string;
  distortion_model: string;
  frame_id: string;
  frame_hash: string;
};

export type PointCloudBody = {
  type: "point_cloud";
  fields: PointField[];
  point_step: number;
  is_bigendian: boolean;
  frame_rate_hz: number;
  frame_id: string;
  frame_hash: string;
};

export type PointField = {
  name: string;
  offset: number;
  datatype:
    | "int8"
    | "uint8"
    | "int16"
    | "uint16"
    | "int32"
    | "uint32"
    | "float32"
    | "float64";
  count: number;
};

export type AudioBody = {
  type: "audio";
  sample_rate_hz: number;
  channels: number;
  sample_format: string;
  channel_layout?: string;
};

export type JointEncodersBody = {
  type: "joint_encoders";
  joint_count: number;
  frame_rate_hz: number;
};

export type SensorCatalogKind =
  | "camera"
  | "point_cloud"
  | "joint_encoders"
  | "audio"
  | "detection"
  | (string & {});

export type SensorCatalogEntry = {
  sensor_id: string;
  sensor_hash: string;
  kind: SensorCatalogKind;
  sensor_entry_json?: string | null;
  frame_entry_json?: string | null;
};

export type SensorCatalog = {
  sensors: SensorCatalogEntry[];
};

export function fetchCatalog(daemonUrl: string): Promise<SensorCatalog | null> {
  const sensors = sdkRuntime.getParticipantSensors(daemonUrl);
  if (sdkRuntime.getParticipant(daemonUrl) == null) return Promise.resolve(null);
  return Promise.resolve({
    sensors: sensors.map((sensor) => ({
      sensor_id: sensor.sensor_id,
      sensor_hash: sensor.sensor_hash,
      kind: sensor.kind,
      sensor_entry_json: sensor.sensor_entry_json ?? null,
      frame_entry_json: sensor.frame_entry_json ?? null,
    })),
  });
}

export async function fetchSensorEntry(
  daemonUrl: string,
  sensorId: string,
  sensorHash: string,
): Promise<SensorEntry | null> {
  const catalog = await fetchCatalog(daemonUrl);
  const entry = catalog?.sensors.find(
    (candidate) => candidate.sensor_id === sensorId && candidate.sensor_hash === sensorHash,
  );
  if (!entry) return null;
  return sensorEntryFromCatalogEntry(entry) ?? null;
}

export function synthEntryFromCatalogKind(
  sensor_id: string,
  kind: SensorCatalogKind,
): SensorEntry | undefined {
  switch (kind) {
    case "camera":
      return {
        sensor_id,
        type: "camera",
        width: 0,
        height: 0,
        frame_rate_hz: 0,
        pixel_format: "jpeg",
        color_space: "srgb",
        intrinsics_model: "unknown",
        distortion_model: "unknown",
        frame_id: "",
        frame_hash: "",
      };
    case "point_cloud":
      return {
        sensor_id,
        type: "point_cloud",
        fields: [],
        point_step: 0,
        is_bigendian: false,
        frame_rate_hz: 0,
        frame_id: "",
        frame_hash: "",
      };
    case "audio":
      return {
        sensor_id,
        type: "audio",
        sample_rate_hz: 0,
        channels: 0,
        sample_format: "unknown",
      };
    case "joint_encoders":
      return {
        sensor_id,
        type: "joint_encoders",
        joint_count: 0,
        frame_rate_hz: 0,
      };
    default:
      return undefined;
  }
}

export function sensorEntryFromCatalogEntry(
  entry: SensorCatalogEntry,
): SensorEntry | undefined {
  if (entry.sensor_entry_json) {
    try {
      const parsed = JSON.parse(entry.sensor_entry_json) as SensorEntry;
      if (isSensorEntry(parsed)) {
        return { ...parsed, sensor_id: entry.sensor_id };
      }
    } catch {
      // Fall through to kind-only synthesis.
    }
  }
  return synthEntryFromCatalogKind(entry.sensor_id, entry.kind);
}

export function describeSensor(e: SensorEntry): string {
  switch (e.type) {
    case "camera":
      return `${e.pixel_format} · ${e.width}x${e.height} @ ${e.frame_rate_hz}fps`;
    case "point_cloud":
      return `point cloud · ${e.fields.length} field${e.fields.length === 1 ? "" : "s"} @ ${e.frame_rate_hz}fps`;
    case "audio":
      return `audio · ${e.sample_format} · ${e.channels}ch @ ${e.sample_rate_hz}Hz`;
    case "joint_encoders":
      return `joint encoders · ${e.joint_count} joints @ ${e.frame_rate_hz}fps`;
  }
}

export function shortHash(h: string): string {
  if (h.length <= 12) return h;
  return `${h.slice(0, 8)}...${h.slice(-3)}`;
}

export type AxisDirection =
  | "forward"
  | "backward"
  | "up"
  | "down"
  | "left"
  | "right";

export type Handedness = "right" | "left";

export type LengthUnit = "meters" | "millimeters" | "centimeters";

export type AxisConvention = {
  x: AxisDirection;
  y: AxisDirection;
  z: AxisDirection;
};

export type FrameRegistryEntry = {
  frame_id: string;
  version?: string;
  handedness: Handedness;
  axes: AxisConvention;
  units: LengthUnit;
};

export type Matrix4 = [
  [number, number, number, number],
  [number, number, number, number],
  [number, number, number, number],
  [number, number, number, number],
];

const identityMatrix: Matrix4 = [
  [1, 0, 0, 0],
  [0, 1, 0, 0],
  [0, 0, 1, 0],
  [0, 0, 0, 1],
];

let geometryInit: Promise<unknown> | null = null;

export async function fetchFrameEntry(
  daemonUrl: string,
  frameId: string,
  frameHash: string,
): Promise<FrameRegistryEntry | null> {
  const catalog = await fetchCatalog(daemonUrl);
  for (const sensor of catalog?.sensors ?? []) {
    if (!sensor.frame_entry_json) continue;
    try {
      const frame = JSON.parse(sensor.frame_entry_json) as FrameRegistryEntry;
      if (frame.frame_id === frameId && sensorMatchesFrameHash(sensor, frameHash)) {
        return frame;
      }
    } catch {
      // Ignore malformed optional frame entries.
    }
  }
  return null;
}

export async function fetchFrameConventionMatrix(
  daemonUrl: string,
  frameId: string,
  frameHash: string,
): Promise<Matrix4 | null> {
  const frame = await fetchFrameEntry(daemonUrl, frameId, frameHash);
  if (!frame) return identityMatrix;
  geometryInit ??= initGeometry();
  await geometryInit;
  const threeFrame: FrameRegistryEntry = {
    frame_id: "three/opengl",
    handedness: "right",
    axes: { x: "right", y: "up", z: "backward" },
    units: "meters",
  };
  return JSON.parse(
    conventionMatrixJson(JSON.stringify(frame), JSON.stringify(threeFrame)),
  ) as Matrix4;
}

export type StreamDescriptor = {
  sensor_id: string;
  sensor_hash: string;
  clock_id: string;
  clock_hash: string;
  frame_id: string;
  frame_hash: string;
};

export function fetchStreamDescriptor(
  peerId: string,
  sensorId: string,
): Promise<StreamDescriptor | null> {
  return Promise.resolve(getRuntimeStreamDescriptor({ peer_id: peerId, sensor_id: sensorId }));
}

function isSensorEntry(entry: SensorEntry): entry is SensorEntry {
  return (
    entry.type === "camera" ||
    entry.type === "point_cloud" ||
    entry.type === "audio" ||
    entry.type === "joint_encoders"
  );
}

function sensorMatchesFrameHash(sensor: SensorCatalogEntry, frameHash: string): boolean {
  if (!frameHash) return true;
  try {
    const entry = sensorEntryFromCatalogEntry(sensor);
    return Boolean(
      entry &&
        "frame_hash" in entry &&
        typeof entry.frame_hash === "string" &&
        entry.frame_hash === frameHash,
    );
  } catch {
    return false;
  }
}
