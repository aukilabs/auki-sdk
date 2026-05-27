// Inline SVG icons. Lucide-style — 2px stroke, rounded joins, monotone.
// `currentColor` so callers control colour via `text-*` Tailwind classes.
//
// Each helper returns a string suitable for innerHTML. Callers wrap in a
// span / set sizing via Tailwind (`w-4 h-4` etc.) on the wrapping element.

import type { SensorEntry } from "./data/registry";

const SVG = (size: number, body: string) =>
  `<svg xmlns="http://www.w3.org/2000/svg" width="${size}" height="${size}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">${body}</svg>`;

// Robot head — for directory cards.
export const iconRobot = (size = 24) =>
  SVG(
    size,
    `
    <rect x="4" y="6" width="16" height="14" rx="2" />
    <line x1="12" y1="2" x2="12" y2="6" />
    <circle cx="12" cy="2" r="0.8" fill="currentColor" />
    <circle cx="9" cy="12" r="1.2" fill="currentColor" />
    <circle cx="15" cy="12" r="1.2" fill="currentColor" />
    <line x1="9" y1="17" x2="15" y2="17" />
  `,
  );

// Camera — head_left_cam etc.
export const iconCamera = (size = 24) =>
  SVG(
    size,
    `
    <path d="M3 7h3l2-2h8l2 2h3v12H3z" />
    <circle cx="12" cy="13" r="3.5" />
  `,
  );

// Point cloud — dotted cube.
export const iconPointCloud = (size = 24) =>
  SVG(
    size,
    `
    <circle cx="6" cy="7" r="0.9" fill="currentColor" stroke="none"/>
    <circle cx="12" cy="5" r="0.9" fill="currentColor" stroke="none"/>
    <circle cx="18" cy="7" r="0.9" fill="currentColor" stroke="none"/>
    <circle cx="6" cy="13" r="0.9" fill="currentColor" stroke="none"/>
    <circle cx="12" cy="11" r="0.9" fill="currentColor" stroke="none"/>
    <circle cx="18" cy="13" r="0.9" fill="currentColor" stroke="none"/>
    <circle cx="9" cy="18" r="0.9" fill="currentColor" stroke="none"/>
    <circle cx="15" cy="18" r="0.9" fill="currentColor" stroke="none"/>
    <circle cx="21" cy="20" r="0.9" fill="currentColor" stroke="none"/>
    <circle cx="3" cy="20" r="0.9" fill="currentColor" stroke="none"/>
  `,
  );

// Pose — TF axes gizmo (three orthogonal arrows).
export const iconPose = (size = 24) =>
  SVG(
    size,
    `
    <line x1="12" y1="20" x2="12" y2="6" />
    <polyline points="9,9 12,6 15,9" />
    <line x1="12" y1="20" x2="20" y2="20" />
    <polyline points="17,17 20,20 17,23" />
    <line x1="12" y1="20" x2="5" y2="14" />
    <polyline points="5,17 5,14 8,14" />
  `,
  );

// World — point cloud + articulated pose in one viewport.
export const iconWorld = (size = 24) =>
  SVG(
    size,
    `
    <circle cx="7" cy="7" r="0.9" fill="currentColor" stroke="none"/>
    <circle cx="13" cy="5" r="0.9" fill="currentColor" stroke="none"/>
    <circle cx="18" cy="9" r="0.9" fill="currentColor" stroke="none"/>
    <circle cx="5" cy="14" r="0.9" fill="currentColor" stroke="none"/>
    <circle cx="16" cy="16" r="0.9" fill="currentColor" stroke="none"/>
    <line x1="12" y1="21" x2="12" y2="10" />
    <polyline points="9,13 12,10 15,13" />
    <line x1="12" y1="21" x2="20" y2="21" />
    <polyline points="17,18 20,21 17,23" />
    <line x1="12" y1="21" x2="6" y2="17" />
    <polyline points="6,20 6,17 9,17" />
  `,
  );

// Microphone.
export const iconMic = (size = 24) =>
  SVG(
    size,
    `
    <rect x="9" y="3" width="6" height="12" rx="3" />
    <path d="M5 11a7 7 0 0 0 14 0" />
    <line x1="12" y1="18" x2="12" y2="22" />
  `,
  );

// Magnifier — quick-search trigger + overlay input.
export const iconSearch = (size = 24) =>
  SVG(
    size,
    `
    <circle cx="11" cy="11" r="7" />
    <line x1="21" y1="21" x2="16.65" y2="16.65" />
  `,
  );

// Gear — settings.
export const iconSettings = (size = 24) =>
  SVG(
    size,
    `
    <circle cx="12" cy="12" r="3" />
    <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09a1.65 1.65 0 0 0-1-1.51 1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09a1.65 1.65 0 0 0 1.51-1 1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06a1.65 1.65 0 0 0 1.82.33h.01a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82v.01a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
  `,
  );

// Database — Discovery service monitor.
export const iconDatabase = (size = 24) =>
  SVG(
    size,
    `
    <ellipse cx="12" cy="5" rx="9" ry="3" />
    <path d="M3 5v14c0 1.7 4 3 9 3s9-1.3 9-3V5" />
    <path d="M3 12c0 1.7 4 3 9 3s9-1.3 9-3" />
  `,
  );

// Refresh — manual monitor refresh.
export const iconRefresh = (size = 24) =>
  SVG(
    size,
    `
    <path d="M21 12a9 9 0 0 1-15.4 6.4L3 16" />
    <path d="M3 16v5h5" />
    <path d="M3 12A9 9 0 0 1 18.4 5.6L21 8" />
    <path d="M21 8V3h-5" />
  `,
  );

// Copy — raw JSON clipboard action.
export const iconCopy = (size = 24) =>
  SVG(
    size,
    `
    <rect x="9" y="9" width="13" height="13" rx="2" />
    <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" />
  `,
  );

// Record — filled center dot inside a ring.
export const iconRecord = (size = 24) =>
  SVG(
    size,
    `
    <circle cx="12" cy="12" r="8" />
    <circle cx="12" cy="12" r="3" fill="currentColor" stroke="none" />
  `,
  );

// Stop — square transport control.
export const iconStop = (size = 24) =>
  SVG(
    size,
    `
    <rect x="7" y="7" width="10" height="10" rx="1.5" fill="currentColor" stroke="none" />
    <rect x="7" y="7" width="10" height="10" rx="1.5" />
  `,
  );

// Plus — add daemon / generic add action.
export const iconPlus = (size = 24) =>
  SVG(
    size,
    `
    <line x1="12" y1="5" x2="12" y2="19" />
    <line x1="5" y1="12" x2="19" y2="12" />
  `,
  );

// Snowflake — freeze-frame on a tile (snapshot a live moment without
// pausing the rest of the stage).
export const iconFreeze = (size = 24) =>
  SVG(
    size,
    `
    <line x1="12" y1="2" x2="12" y2="22" />
    <line x1="2" y1="12" x2="22" y2="12" />
    <line x1="4.5" y1="4.5" x2="19.5" y2="19.5" />
    <line x1="19.5" y1="4.5" x2="4.5" y2="19.5" />
    <polyline points="9,4 12,2 15,4" />
    <polyline points="9,20 12,22 15,20" />
    <polyline points="4,9 2,12 4,15" />
    <polyline points="20,9 22,12 20,15" />
  `,
  );

// Download — snapshot the current frame to disk.
export const iconDownload = (size = 24) =>
  SVG(
    size,
    `
    <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
    <polyline points="7,10 12,15 17,10" />
    <line x1="12" y1="15" x2="12" y2="3" />
  `,
  );

// Info — circled lowercase i. Inspector trigger.
export const iconInfo = (size = 24) =>
  SVG(
    size,
    `
    <circle cx="12" cy="12" r="9" />
    <line x1="12" y1="11" x2="12" y2="16" />
    <line x1="12" y1="8" x2="12" y2="8" stroke-linecap="round" />
  `,
  );

// Close — × glyph for tile close + overlay dismiss.
export const iconClose = (size = 24) =>
  SVG(
    size,
    `
    <line x1="6" y1="6" x2="18" y2="18" />
    <line x1="18" y1="6" x2="6" y2="18" />
  `,
  );

// Generic sensor — fallback for unknown modalities.
export const iconSensor = (size = 24) =>
  SVG(
    size,
    `
    <circle cx="12" cy="12" r="9" />
    <circle cx="12" cy="12" r="4.5" />
    <circle cx="12" cy="12" r="1.5" fill="currentColor" stroke="none"/>
  `,
  );

export type SensorType =
  | "camera"
  | "pointcloud"
  | "pose"
  | "world"
  | "mic"
  | "sensor";

/** @deprecated Substring-matching heuristic. Prefer
 * `sensorTypeFromEntry(sensor_id, entry)` — the SDK registry kind is
 * canonical. Bracketbot's depth_camera (a point cloud sensor whose id
 * contains "cam") is mis-classified here as RGB video. Still used by
 * the directory dashboard card's thumbnail picker; track follow-up
 * cleanup on the project board. */
export function sensorTypeFromId(sensor_id: string): SensorType {
  const lower = sensor_id.toLowerCase();
  if (lower.includes("cam")) return "camera";
  if (lower.includes("pointcloud") || lower.includes("pcd") || lower.includes("depth")) {
    return "pointcloud";
  }
  if (lower.includes("pose") || lower.includes("/tf") || lower.includes("joint")) return "pose";
  if (lower.includes("mic") || lower.includes("audio")) return "mic";
  return "sensor";
}

/** Map a SDK-declared `SensorEntry` to Park's UI-facing icon type.
 *
 * Source of truth for the sensor type — the registry kind, not a
 * substring guess from the sensor_id. Bracketbot's pointcloud sensor
 * is named `<...>/depth_camera`; the old heuristic short-circuited to
 * `"camera"` on the `"cam"` substring before ever checking for the
 * pointcloud markers, so the depth feed got routed (and iconified)
 * as RGB video.
 *
 * Two sensor flavours don't have a registry kind today and are
 * routed by sensor_id suffix:
 * - `<node>/joint_encoders` — K1 articulated pose (Park's `k1_pose`
 *   tile, fed by `auki-network`'s `JointEncoders` sensor body).
 * - `<node>/pose` — Park-synthesized past-session row carrying the
 *   on-disk TF tree (no SDK kind; Park's `pose` tile renders it).
 *
 * Returns `"sensor"` when the entry hasn't resolved yet — the live
 * registry fetch is async on first open. */
export function sensorTypeFromEntry(
  sensor_id: string,
  entry: SensorEntry | undefined,
): SensorType {
  if (sensor_id === "world") return "world";
  if (sensor_id.endsWith("/joint_encoders")) return "pose";
  if (sensor_id.endsWith("/pose")) return "pose";
  if (!entry) return "sensor";
  switch (entry.type) {
    case "camera":
      return "camera";
    case "point_cloud":
      return "pointcloud";
    case "audio":
      return "mic";
    case "joint_encoders":
      return "pose";
  }
}

export function sensorTypeFromCatalog(
  sensor_id: string,
  kind: string | undefined,
): SensorType {
  if (sensor_id === "world") return "world";
  switch (kind) {
    case "camera":
      return "camera";
    case "point_cloud":
      return "pointcloud";
    case "audio":
      return "mic";
    case "joint_encoders":
      return "pose";
    default:
      return sensorTypeFromId(sensor_id);
  }
}

export function iconForSensorType(t: SensorType, size = 24): string {
  switch (t) {
    case "camera":
      return iconCamera(size);
    case "pointcloud":
      return iconPointCloud(size);
    case "pose":
      return iconPose(size);
    case "world":
      return iconWorld(size);
    case "mic":
      return iconMic(size);
    case "sensor":
      return iconSensor(size);
  }
}
