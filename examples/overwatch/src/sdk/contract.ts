export type SensorKind =
  | "audio"
  | "camera"
  | "point_cloud"
  | "joint_encoders"
  | "detection"
  | (string & {});

export type SensorSummary = {
  sensor_id: string;
  sensor_hash: string;
  kind: SensorKind;
  label?: string;
  sensor_entry_json?: string | null;
  frame_entry_json?: string | null;
};

export type SensorStreamEntry = {
  timestamp_ns?: number;
  seq?: number;
  payload: number[] | Uint8Array;
};

export type SensorSource =
  | {
      kind: "generated-bytes";
      frames: number[][];
      interval_ms?: number;
      start_timestamp_ns?: number;
    }
  | Iterable<SensorStreamEntry>
  | AsyncIterable<SensorStreamEntry>;

export type StreamHandle = {
  nextMessage(): Promise<unknown>;
  close?(): Promise<void> | void;
};

export type PeerDebugState = Record<string, unknown>;

export type PeerSnapshot = {
  selfPeerId: string;
  domainName: string | null;
  managerPeerId: string | null;
  role: "manager" | "member" | "idle";
  participants: Array<{
    peer_id: string;
    name?: string;
    app?: string;
    is_self?: boolean;
    is_manager?: boolean;
    connected?: boolean;
    multiaddrs?: string[];
    sensors?: SensorSummary[];
  }>;
};

export type OverwatchPeer = {
  readonly peerId: string;
  createOrJoin(input: { discoveryUrl: string; clusterName: string }): Promise<void>;
  observeParticipants(cb: (snapshot: PeerSnapshot) => void): () => void;
  declareSensors(sensors: SensorSummary[]): Promise<void>;
  publishSensor(sensorId: string, source: SensorSource): Promise<void>;
  subscribeToSensor(peerId: string, sensorId: string): Promise<StreamHandle>;
  debugState(): PeerDebugState;
};
