import { iconRecord, iconStop } from "../../icons";
import type { InspectorAction, InspectorSection } from "../../shell/inspectorDrawer";
import { toast } from "../../shell/toast";
import {
  activeRecordingForSensor,
  isActiveRecording,
  startIntentRecording,
  stopIntentRecording,
  subscribeRecordings,
  type RecordingWire,
} from "../../data/recordings";

export type RecordingInspectorControl = {
  actions(): InspectorAction[];
  section(): InspectorSection;
  dispose(): void;
};

export function makeRecordingInspectorControl(opts: {
  peerId: string;
  sensorId: string;
  onChange: () => void;
}): RecordingInspectorControl {
  let recording: RecordingWire | null = null;
  let busy = false;
  const dispose = subscribeRecordings((rows) => {
    recording = activeRecordingForSensor(rows, opts.peerId, opts.sensorId);
    opts.onChange();
  });

  const toggle = async () => {
    if (busy) return;
    busy = true;
    opts.onChange();
    try {
      if (recording && isActiveRecording(recording)) {
        const stopped = await stopIntentRecording(recording.id);
        recording = stopped;
        toast.info("Recording stopped");
      } else {
        recording = await startIntentRecording(opts.peerId, opts.sensorId);
        toast.info("Recording started");
      }
    } catch (e) {
      const detail = e instanceof Error ? e.message : String(e);
      toast.error(detail);
    } finally {
      busy = false;
      opts.onChange();
    }
  };

  return {
    actions() {
      const active = recording ? isActiveRecording(recording) : false;
      return [
        {
          id: "recording-toggle",
          label: busy ? "Working" : active ? "Stop" : "Record",
          icon: active ? iconStop(13) : iconRecord(13),
          tone: active ? "danger" : "accent",
          disabled: busy || recording?.status === "stopping",
          title: active
            ? "Stop this intent recording"
            : "Start an intent recording for this sensor",
          onClick: toggle,
        },
      ];
    },
    section() {
      if (!recording) {
        return {
          title: "Recording",
          rows: [
            {
              key: "status",
              value: "idle",
              mono: false,
              dim: true,
            },
          ],
        };
      }
      return {
        title: "Recording",
        rows: [
          { key: "status", value: recording.status, mono: false },
          { key: "frames", value: String(recording.frames_written) },
          { key: "bytes", value: formatBytes(recording.bytes_written) },
          {
            key: "started",
            value: formatUnixNs(recording.started_at_unix_ns),
            mono: false,
          },
          {
            key: "path",
            value: recording.log_path,
            mono: false,
          },
          {
            key: "render",
            value: recording.render_path ?? "-",
            mono: false,
            dim: !recording.render_path,
          },
          {
            key: "render_error",
            value: recording.render_error ?? "-",
            mono: false,
            dim: !recording.render_error,
          },
          {
            key: "error",
            value: recording.last_error ?? "-",
            mono: false,
            dim: !recording.last_error,
          },
        ],
      };
    },
    dispose,
  };
}

function formatUnixNs(ns: number): string {
  if (!Number.isFinite(ns) || ns <= 0) return "-";
  return new Date(Math.floor(ns / 1_000_000)).toISOString();
}

function formatBytes(n: number): string {
  if (!Number.isFinite(n) || n < 0) return "-";
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(1)} MB`;
  return `${(n / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}
