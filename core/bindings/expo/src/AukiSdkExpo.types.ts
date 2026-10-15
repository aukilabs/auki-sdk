export type AukiDiscoveryModeName = "DiscoverOnly" | "DiscoverAndAdvertise";

/** `OutboundOnly` dials remote relays without a local `/relay-bookings` call. */
export type AukiPeerReachabilityModeName = "OutboundOnly" | "RelayBacked";

export type AukiDomainInfo = {
  id: string;
  name?: string | null;
  description?: string | null;
  organizationId?: string | null;
};

export type AukiDiscoveryCandidateInfo = {
  peerId: string;
  routes: string[];
  servedProtocols: string[];
  expiresAt: string;
  source: string;
  subjectId?: string | null;
  peerType?: string | null;
};

/** Advertised peer id and route. This is the exact dial. */
export type PeerRoute = {
  peerId: string;
  route: string;
};

/** @internal Bridge name for {@link PeerRoute}. */
export type AukiExactTarget = PeerRoute;

/** Public-client host PKCE result. The SDK becomes the sole refresh owner. */
export type ZitadelSessionCredentials = {
  accessToken: string;
  refreshToken: string;
  clientId: string;
  issuer: string;
  /** RFC 3339, preserved as text; null/undefined means unknown. */
  accessTokenExpiresAt?: string | null;
};

export type AukiServiceEnvironment = {
  apiBaseUrl: string;
  ddsBaseUrl: string;
  dmsBaseUrl: string;
};

export type AukiAuthFailureCode = "authentication_required" | "configuration" |
  "authorization_denied" | "persistence" | "transient" | "cancelled" | "closed";

export type AukiAuthError = Error & { readonly code: AukiAuthFailureCode };

export type DomainQuery = {
  organization?: string;
  domainServerId?: string;
  limit?: number;
  offset?: number;
};

export type DomainSummary = {
  id: string;
  name: string;
  organizationId: string | null;
};

export type DomainPage = {
  domains: DomainSummary[];
  total: number;
  limit: number;
  offset: number;
};

export type PortalDomain = DomainSummary & {
  isDefault: boolean;
  addedToDomainAt: string;
};

export type Portal = {
  id: string;
  shortId: string;
  name: string;
  size: number;
  organizationId: string | null;
  defaultDomainId: string | null;
  redirectUrl: string | null;
  createdAt: string;
  updatedAt: string;
};

export type PortalPose = {
  id: string;
  shortId: string;
  domainId: string;
  reportedSize: number;
  px: number;
  py: number;
  pz: number;
  rx: number;
  ry: number;
  rz: number;
  rw: number;
  latitude: number | null;
  longitude: number | null;
  altitude: number | null;
  verticalAccuracy: number | null;
  horizontalAccuracy: number | null;
  gpsTimestamp: number | null;
  scannerDeviceId: string;
  scannerDeviceName: string;
  scannerDeviceModel: string;
  placedAt: string;
};

export type DataMetadata = {
  id: string;
  domainId: string;
  name: string;
  dataType: string;
  size: number;
  createdAt: string;
  updatedAt: string;
};

export type DataQuery = { ids?: string[]; name?: string; dataType?: string };
export type DataTarget =
  | { name: string; dataType: string; id?: never }
  | { id: string; name?: never; dataType?: never };
export type TransferOptions = { maxBytes?: number; maxChunkBytes?: number };
export type DataSource = (
  maximumBytes: number,
  signal: AbortSignal,
) => Uint8Array | Promise<Uint8Array>;
export type DataSink = (
  chunk: Uint8Array,
  signal: AbortSignal,
) => void | Promise<void>;
export type JsonPrimitive = string | number | boolean | null;
export type JsonValue = JsonPrimitive | JsonValue[] | { [key: string]: JsonValue };

export type JobMode = "public" | "dedicated";
export type JobStatus = "pending" | "running" | "completed" | "failed" | "canceled";

export type JobTaskSpec = {
  label: string;
  stage: string;
  capability: string;
  mode?: JobMode;
  capabilityFilters?: Record<string, string>;
  priority?: number;
  inputsCids?: string[];
  outputsPrefix?: string;
  meta?: { [key: string]: JsonValue };
  maxAttempts?: number;
};

export type JobEdge = { from: string; to: string };

export type JobSpec = {
  label: string;
  priority?: number;
  meta?: { [key: string]: JsonValue };
  tasks: JobTaskSpec[];
  edges?: JobEdge[];
};

export type JobListQuery = {
  limit?: number;
  cursor?: string;
  status?: JobStatus;
  capabilities?: string[];
  matchAllCapabilities?: boolean;
};

export type JobEstimateTask = {
  label: string;
  stage: string;
  capability: string;
  mode: JobMode;
  billingUnits: string;
  estimatedCreditCost: string;
};

export type JobEstimate = { total: string; tasks: JobEstimateTask[] };

export type JobRecord = {
  id: string;
  label: string;
  domainId: string;
  status: JobStatus;
  priority: number;
  createdAt: string;
  updatedAt: string;
  organizationId: string | null;
  meta: JsonValue;
  creditLockId: string | null;
  creditLockAmount: string | null;
  creditLockedAt: string | null;
  creditReleasedAt: string | null;
};

export type JobTaskSummary = {
  queued: number;
  leased: number;
  running: number;
  completed: number;
  failed: number;
  canceled: number;
};

export type JobListItem = { job: JobRecord; tasksSummary: JobTaskSummary };

export type JobPage = {
  items: JobListItem[];
  nextCursor: string | null;
};

export type JobTaskStatus = "queued" | "leased" | "running" | "completed" | "failed" | "canceled";

export type JobTask = {
  id: string;
  jobId: string;
  label: string;
  stage: string;
  capability: string;
  capabilityFilters: Record<string, string>;
  status: JobTaskStatus;
  depsRemaining: number;
  priority: number;
  inputsCids: string[];
  outputsPrefix: string | null;
  organizationId: string | null;
  attempts: number;
  maxAttempts: number;
  leaseExpiresAt: string | null;
  reservedBy: string | null;
  meta: JsonValue;
  cancelRequestedAt: string | null;
  lastHeartbeatAt: string | null;
  createdAt: string;
  updatedAt: string;
  mode: JobMode;
  billingUnits: string;
  estimatedCreditCost: string | null;
  debitedAmount: string | null;
  debitedAt: string | null;
};

export type JobReceipt = {
  id: string;
  jobId: string;
  taskId: string;
  nodeId: string | null;
  outputs: string[];
  meta: JsonValue;
  createdAt: string;
};

export type JobDetails = {
  job: JobRecord;
  tasksSummary: JobTaskSummary;
  tasks: JobTask[];
  receipts: JobReceipt[];
};

export type JobCancellation = { id: string; status: JobStatus; updatedAt: string };

export type AukiSdkExpoModuleEvents = {
  /** Internal request/ACK bridge. Contains identifiers only, never tokens. */
  onZitadelSaveRequested: (request: { sessionId: string; requestId: string }) => void;
};

export interface FleetQuery { capabilities?: string[]; matchAllCapabilities?: boolean }
export interface ComputePoolQuery extends FleetQuery { mode: JobMode }
export type FleetPresence = "online" | "offline" | "unknown";
export type FleetWorkState = "idle" | "busy" | "unknown";
export interface FleetActivity {
  workerId: string; jobId: string; taskId: string;
  taskStatus: JobTaskStatus; jobStatus: JobStatus; mode: JobMode; capability: string;
  leaseExpiresAt: string | null; lastHeartbeatAt: string | null;
  updatedAt: string; observedAt: string;
}
export interface FleetMachine {
  kind: "robot" | "compute"; id: string; organizationId: string; name: string;
  capabilities: string[]; mode: string; association: "assigned" | "active_task" | "candidate";
  presence: FleetPresence; providerStatus: string; presenceObservedAt: string;
  lastSeenAt: string | null; activeLeaseExpiresAt: string | null;
  workState: FleetWorkState; workObservedAt: string | null; activity: FleetActivity[];
}
export interface FleetSourceReport {
  source: "robots" | "nodes" | "jobs" | "busy";
  state: "complete" | "partial" | "denied" | "unsupported" | "unavailable";
  observedAt: string; codes: string[]; httpStatus: number | null;
}
export interface FleetSnapshot {
  domainId: string; view: "domain" | "compute_pool"; observedAt: string;
  machines: FleetMachine[]; unresolvedActivity: FleetActivity[];
  sources: FleetSourceReport[]; complete: boolean;
}

export type PortalPage = { items: Portal[]; nextCursor: string | null; paginated: boolean };
export type PortalDomainPage = { items: PortalDomain[]; nextCursor: string | null; paginated: boolean };
