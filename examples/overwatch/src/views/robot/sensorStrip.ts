// Sensor strip — bottom rail. Each chip is a toggle: clicking adds /
// removes that sensor's tile from the stage. Sensors come from the
// active session — Live's sensors are derived from the daemon's
// `/api/sensor_logs?session_id=current` poll; past sessions list
// whatever sensors that on-disk session contains.
//
// Styling matches the original split-views commit (1f396b4): a top
// border, a "SENSORS" kicker label *above* the chip row, then a row of
// 176×80 chips with sensor icon + name + ("type · N logs") subtitle.

import { escapeHtml } from "../../util/escape";
import type { SensorLog } from "../../data/sensorLogs";
import type { SensorEntry } from "../../data/registry";
import { iconForSensorType, sensorTypeFromEntry } from "../../icons";

export type SensorRow = {
  sensor_id: string;
  sensor_hash?: string;
  sensor_logs: SensorLog[];
  /** Sensor registry entry from the SDK (Control API v1's
   * content-addressed `(sensor_id, sensor_hash)` lookup). Drives tile
   * routing in `pushTilesToStage` and the default-toggle seed in
   * `pickDefaultSensor` — both must use the declared `entry.type`
   * rather than guessing from the sensor_id string. Undefined while
   * the live-session registry fetch is in flight. */
  entry?: SensorEntry;
};

export type SensorStripHandle = {
  el: HTMLElement;
  setSensors(rows: SensorRow[]): void;
  setToggled(toggled: Set<string>): void;
  onToggle(cb: (sensor_id: string) => void): void;
  getSensors(): SensorRow[];
};

export function makeSensorStrip(): SensorStripHandle {
  const el = document.createElement("footer");
  el.className =
    "row-start-2 col-span-2 border-t border-paper/10 bg-ink px-4 pt-2 pb-3 shrink-0";
  el.innerHTML = `
    <div class="text-rule text-[11px] uppercase tracking-[0.2em] mb-2" style="font-family: var(--font-display)">Sensors</div>
    <div class="flex items-center gap-2 overflow-x-auto pb-1" data-region="sensors"></div>
  `;

  const slot = el.querySelector('[data-region="sensors"]') as HTMLElement;

  let sensors: SensorRow[] = [];
  let toggled: Set<string> = new Set();
  const toggleListeners = new Set<(id: string) => void>();

  const render = () => {
    slot.replaceChildren();
    if (sensors.length === 0) {
      const empty = document.createElement("div");
      empty.className =
        "h-20 px-4 flex items-center gap-3 border border-dashed border-paper/10 rounded-md text-rule/70";
      empty.innerHTML = `
        <span class="w-1.5 h-1.5 rounded-full bg-rule/40 animate-pulse shrink-0"></span>
        <div class="min-w-0">
          <div class="text-paper/75 text-xs">No sensors in this session yet</div>
          <div class="text-rule/55 text-[11px] mt-0.5">they appear here as the device starts streaming</div>
        </div>
      `;
      slot.appendChild(empty);
      return;
    }
    let idx = 0;
    for (const s of sensors) {
      slot.appendChild(chip(s, toggled.has(s.sensor_id), idx++));
    }
  };

  const chip = (s: SensorRow, isOn: boolean, idx: number): HTMLElement => {
    const t = document.createElement("button");
    const base =
      "shrink-0 w-44 h-20 rounded-md border px-3 py-2 text-left flex items-center gap-3 transition-colors relative";
    const styling = isOn
      ? "border-accent bg-ink-alt"
      : "border-paper/10 hover:border-paper/30 bg-ink-alt/40";
    t.className = `${base} ${styling}`;
    t.title = isOn ? "Click to remove from stage" : "Click to add to stage";

    const type = sensorTypeFromEntry(s.sensor_id, s.entry);
    const short = s.sensor_id.split("/").pop() ?? s.sensor_id;
    const iconColor = isOn ? "text-accent" : "text-paper/60";
    const logCount = s.sensor_logs.length;
    const shortcutHint = idx < 9
      ? `<span class="absolute top-1 right-2 text-[10px] text-rule/40 font-mono">${idx + 1}</span>`
      : "";

    t.innerHTML = `
      <div class="${iconColor} shrink-0">${iconForSensorType(type, 28)}</div>
      <div class="flex-1 min-w-0">
        <div class="text-paper text-xs font-medium truncate">${escapeHtml(short)}</div>
        <div class="text-rule text-[11px] truncate mt-0.5">${escapeHtml(type)}${logCount ? ` · ${logCount} log${logCount === 1 ? "" : "s"}` : ""}</div>
      </div>
      ${shortcutHint}
    `;
    t.addEventListener("click", () =>
      toggleListeners.forEach((cb) => cb(s.sensor_id)),
    );
    return t;
  };

  render();

  return {
    el,
    setSensors(rows: SensorRow[]) {
      sensors = rows;
      render();
    },
    setToggled(next: Set<string>) {
      toggled = next;
      render();
    },
    onToggle(cb: (id: string) => void) {
      toggleListeners.add(cb);
    },
    getSensors() {
      return sensors;
    },
  };
}

