import { makeTileChrome } from "./chrome";
import type { TileHandle } from "../tile";

export function makeWorldTile(
  spec: {
    sensor_id: "world";
    daemon_url: string;
    peer_id: string;
    point_cloud_sensor_id: string;
    joint_sensor_id: string;
  },
  opts: { onClose: () => void },
): TileHandle & { requestRender(): void } {
  const chrome = makeTileChrome({
    sensor_id: spec.sensor_id,
    type: "world",
    onClose: opts.onClose,
  });

  chrome.body.innerHTML = `
    <div class="absolute inset-0 flex flex-col items-center justify-center gap-1 text-rule pointer-events-none">
      <span class="text-[10px] uppercase tracking-[0.2em] text-paper/70">unsupported</span>
      <span class="text-[10px] text-rule/70">World view requires browser SDK FK support</span>
    </div>
  `;
  chrome.bottomInfo.innerHTML = `
    <span class="truncate" title="${spec.point_cloud_sensor_id} + ${spec.joint_sensor_id}">world</span>
    <span class="text-rule/70 shrink-0">unsupported</span>
  `;

  return {
    el: chrome.el,
    dispose() {},
    toggleFreeze() {},
    snapshot() {},
    close() {
      opts.onClose();
    },
    isFrozen() {
      return false;
    },
    sensorId() {
      return spec.sensor_id;
    },
    setSensorLogs() {},
    requestRender() {},
  };
}
