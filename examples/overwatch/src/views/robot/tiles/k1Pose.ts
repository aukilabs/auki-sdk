import { makeTileChrome } from "./chrome";
import type { TileHandle } from "../tile";

export function makeK1PoseTile(
  spec: {
    sensor_id: string;
    daemon_url: string;
    peer_id: string;
  },
  opts: { onClose: () => void },
): TileHandle & { requestRender(): void } {
  const chrome = makeTileChrome({
    sensor_id: spec.sensor_id,
    type: "pose",
    onClose: opts.onClose,
  });

  chrome.body.innerHTML = `
    <div class="absolute inset-0 flex flex-col items-center justify-center gap-1 text-rule pointer-events-none">
      <span class="text-[10px] uppercase tracking-[0.2em] text-paper/70">unsupported</span>
      <span class="text-[10px] text-rule/70">K1 pose requires a browser SDK FK source</span>
    </div>
  `;
  chrome.bottomInfo.innerHTML = `
    <span class="truncate" title="${spec.sensor_id}">${spec.sensor_id}</span>
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

export function k1PoseDropDelta(_previousSeq: number | null, _nextSeq: number): number {
  return 0;
}
