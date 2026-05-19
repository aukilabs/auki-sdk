export type PeerId = string;
export type SensorId = string;
export type DomainName = string;

export type Result<T> =
  | { ok: true; value: T }
  | { ok: false; error: PeerError };

export type PeerError = {
  code:
    | "transport_unavailable"
    | "discovery_unreachable"
    | "domain_list_failed"
    | "domain_create_failed"
    | "domain_join_failed"
    | "domain_leave_failed"
    | "sensor_publish_failed"
    | "sensor_subscribe_failed"
    | "sensor_unsubscribe_failed"
    | "unsupported"
    | "unknown";
  message: string;
};

export type DomainSummary = {
  name: DomainName;
  managerPeerId?: PeerId;
  peerCount?: number;
};

export type SensorKind =
  | "microphone"
  | "rgb_camera"
  | "point_cloud"
  | "joint_pose"
  | "unknown";

export type SensorSummary = {
  id: SensorId;
  kind: SensorKind;
  label: string;
  publishable: boolean;
  subscribable: boolean;
};

export type StreamState =
  | "off"
  | "connecting"
  | "connected"
  | "reconnecting"
  | "failed"
  | "stalled";

export type MediaPresence = {
  micAvailable: boolean;
  micPublicationEnabled: boolean;
  micCaptureHealthy: boolean;
  listeningToPeerId: PeerId | null;
  listeningToSensorId: SensorId | null;
  playbackHealthy: boolean;
  selectedRemoteStreamState: StreamState;
  lastFrameUnixMs: number | null;
  inputLevel: number | null;
  outputLevel: number | null;
};

export type Participant = {
  peerId: PeerId;
  appId: string;
  displayName: string;
  isSelf: boolean;
  connected: boolean;
  sensors: SensorSummary[];
  mediaPresence: MediaPresence;
};

export type PeerSnapshot = {
  selfPeerId: PeerId;
  domainName: DomainName | null;
  participants: Participant[];
  managerPeerId: PeerId | null;
  electionState: "unknown" | "stable" | "degraded";
};

export type Unsubscribe = () => void;

export type BrowserDomainPeer = {
  getSelfPeerId(): Promise<PeerId>;
  listDomains(discoveryUrl: string): Promise<Result<DomainSummary[]>>;
  createDomain(discoveryUrl: string, domainName: DomainName): Promise<Result<void>>;
  joinDomain(discoveryUrl: string, domainName: DomainName): Promise<Result<void>>;
  leaveDomain(): Promise<Result<void>>;
  observeParticipants(onSnapshot: (snapshot: PeerSnapshot) => void): Unsubscribe;
  setParticipantMetadata(metadata: { appId: string; displayName: string }): Promise<Result<void>>;
  declareLocalSensors(sensors: SensorSummary[]): Promise<Result<void>>;
  setSensorPublication(sensorId: SensorId, enabled: boolean): Promise<Result<void>>;
  subscribeToSensor(peerId: PeerId, sensorId: SensorId): Promise<Result<void>>;
  unsubscribeFromSensor(peerId: PeerId, sensorId: SensorId): Promise<Result<void>>;
};

export type BrowserDomainPeerFactory = {
  createPeer(): Promise<BrowserDomainPeer>;
};
