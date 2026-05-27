import { sdkRuntime } from "../sdk/runtime";

export type SensorLog = {
  sensor_log_id: string;
  session_id: string;
  sensor_id: string;
  sensor_hash: string;
  clock_id: string;
  clock_hash: string;
  retention_ns: number;
  duration_ns: number;
  started_at_ns: number;
  stopped_at_ns: number | null;
};

export type DaemonSensorLogs = {
  sensor_logs: SensorLog[];
};

type Listener = (state: DaemonSensorLogs | null) => void;

export function subscribeSensorLogs(url: string, cb: Listener): () => void {
  return sdkRuntime.subscribeCluster(() => {
    const sensors = sdkRuntime.getParticipantSensors(url);
    if (sdkRuntime.getParticipant(url) == null) {
      cb(null);
      return;
    }
    cb({
      sensor_logs: sensors.map((sensor) => ({
        sensor_log_id: `${url}/${sensor.sensor_id}`,
        session_id: `${url}/browser-session`,
        sensor_id: sensor.sensor_id,
        sensor_hash: sensor.sensor_hash,
        clock_id: `${sensor.sensor_id}/clock`,
        clock_hash: `${sensor.sensor_hash}:clock`,
        retention_ns: 0,
        duration_ns: 0,
        started_at_ns: 1,
        stopped_at_ns: null,
      })),
    });
  });
}
