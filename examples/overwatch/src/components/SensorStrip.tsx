import { useEffect, useState } from "react";

import type { SensorSummary } from "../sdk/contract";

export function SensorStrip({
  peerId,
  sensors,
  onToggleSensor,
}: {
  peerId: string;
  sensors: SensorSummary[];
  onToggleSensor: (sensor: SensorSummary, enabled: boolean) => void;
}) {
  const [enabled, setEnabled] = useState<Set<string>>(() => loadToggled(peerId));

  useEffect(() => {
    setEnabled(loadToggled(peerId));
  }, [peerId]);

  function toggle(sensor: SensorSummary) {
    setEnabled((current) => {
      const next = new Set(current);
      const willEnable = !next.has(sensor.sensor_id);
      if (willEnable) next.add(sensor.sensor_id);
      else next.delete(sensor.sensor_id);
      saveToggled(peerId, next);
      onToggleSensor(sensor, willEnable);
      return next;
    });
  }

  return (
    <div className="flex min-h-12 flex-wrap items-center gap-2 border-t border-line bg-ink/50 px-3 py-2">
      {sensors.map((sensor) => (
        <button
          key={sensor.sensor_id}
          className={[
            "rounded-control border px-2 py-1 text-xs",
            enabled.has(sensor.sensor_id)
              ? "border-signal bg-signal text-ink"
              : "border-line text-slate-300 hover:border-signal",
          ].join(" ")}
          onClick={() => toggle(sensor)}
          type="button"
        >
          {sensor.label ?? sensor.sensor_id}
        </button>
      ))}
      {sensors.length === 0 ? <span className="text-xs text-slate-500">No sensors</span> : null}
    </div>
  );
}

function storageKey(peerId: string): string {
  return `auki:overwatch:toggled:v1:${peerId}`;
}

function loadToggled(peerId: string): Set<string> {
  try {
    const parsed = JSON.parse(globalThis.localStorage?.getItem(storageKey(peerId)) ?? "[]");
    return new Set(Array.isArray(parsed) ? parsed.filter((value) => typeof value === "string") : []);
  } catch {
    return new Set();
  }
}

function saveToggled(peerId: string, value: Set<string>) {
  globalThis.localStorage?.setItem(storageKey(peerId), JSON.stringify([...value]));
}
