// Stage tile dispatcher. The actual tile makers live alongside in
// `tiles/`:
//
//   - `tiles/liveVideo.ts`  — Live video tile (preview poll, freeze).
//   - `tiles/pointCloud.ts` — Three.js point-cloud tile.
//   - `tiles/k1Pose.ts`     — K1 articulated-pose viewport.
//   - `tiles/world.ts`      — Composite point-cloud + K1 pose viewport.
//   - `tiles/chrome.ts`     — Shared chrome (top chip, top close, bottom
//                              bar) used by every tile type so the frame
//                              is consistent.

import type { SensorLog } from "../../data/sensorLogs";
import type { SensorEntry } from "../../data/registry";
import { makeLiveVideoTile } from "./tiles/liveVideo";
import { makePointCloudTile } from "./tiles/pointCloud";
import { makeK1PoseTile } from "./tiles/k1Pose";
import { makeWorldTile } from "./tiles/world";

export type TileSpec =
  | {
      kind: "video";
      sensor_id: string;
      sensor_hash?: string;
      daemon_url: string;
      /// Producer's libp2p PeerId — required for the live tile's
      /// `subscribePreview` call against `/api/streams/<peer_id>/
      /// <sensor_id>/latest.jpg`. Sourced from the daemon's `/api/info`
      /// in `robot/index.ts`.
      peer_id: string;
      /// SDK-declared registry entry for this sensor — drives the
      /// tile chip's icon + label. Set on the route into the tile
      /// by `robot/index.ts` after the entry has been fetched.
      entry?: SensorEntry;
    }
  | {
      kind: "point_cloud";
      sensor_id: string;
      daemon_url: string;
      peer_id: string;
      entry?: SensorEntry;
    }
  | {
      // K1 articulated pose viewport (sawslin Lane D step 5b). Subscribes
      // to Park's `/api/k1/pose/:peer_id` WebSocket — server-side FK on
      // every boosterapp PoseStream frame, browser renders the URDF's
      // STLs. `peer_id` selects which boosterapp producer's pose to
      // render; `sensor_id` (e.g. "K1-WALK01/joint_encoders") is the
      // producer-scoped stream id, used for chip labeling.
      kind: "k1_pose";
      sensor_id: string;
      daemon_url: string;
      peer_id: string;
    }
  | {
      // Virtual "world" sensor. It appears only when the producer has
      // both a point-cloud stream and a JointEncoders stream, and renders
      // the two into one shared Three.js scene.
      kind: "world";
      sensor_id: "world";
      daemon_url: string;
      peer_id: string;
      point_cloud_sensor_id: string;
      joint_sensor_id: string;
    };

export type TileHandle = {
  el: HTMLElement;
  dispose(): void;
  toggleFreeze(): void;
  snapshot(): void;
  close(): void;
  isFrozen(): boolean;
  sensorId(): string;
  /** Update which sensor logs the daemon reports. The tile may use
   * this to derive buffer / recording state for its own UI; tiles
   * that don't care wire it as a no-op. */
  setSensorLogs(logs: SensorLog[]): void;
};

export function makeTile(
  spec: TileSpec,
  opts: { onClose: () => void },
): TileHandle {
  switch (spec.kind) {
    case "video":
      return makeLiveVideoTile(spec, opts);
    case "point_cloud":
      return makePointCloudTile(spec, opts);
    case "k1_pose":
      return makeK1PoseTile(spec, opts);
    case "world":
      return makeWorldTile(spec, opts);
  }
}
