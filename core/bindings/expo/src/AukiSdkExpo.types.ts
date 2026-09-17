export type AukiDiscoveryModeName = "DiscoverOnly" | "DiscoverAndAdvertise";

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

export type AukiExactTarget = {
  peerId: string;
  route: string;
};

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
  organization_id: string | null;
};

export type DomainPage = {
  domains: DomainSummary[];
  total: number;
  limit: number;
  offset: number;
};

export type PortalDomain = DomainSummary & {
  is_default: boolean;
  added_to_domain_at: string;
};

export type Portal = {
  id: string;
  short_id: string;
  name: string;
  size: number;
  organization_id: string | null;
  default_domain_id: string | null;
  redirect_url: string | null;
  created_at: string;
  updated_at: string;
};

export type PortalPose = {
  id: string;
  short_id: string;
  domain_id: string;
  reported_size: number;
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
  vertical_accuracy: number | null;
  horizontal_accuracy: number | null;
  gps_timestamp: number | null;
  scanner_device_id: string;
  scanner_device_name: string;
  scanner_device_model: string;
  placed_at: string;
};

export type DataMetadata = {
  id: string;
  domain_id: string;
  name: string;
  data_type: string;
  size: number;
  created_at: string;
  updated_at: string;
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
export type DomainDataFailureKind =
  | "cancelled"
  | "closed"
  | "timeout"
  | "auth"
  | "http"
  | "input"
  | "response"
  | "limit"
  | "callback"
  | "cleanup"
  | "transport";
export type DomainDataError = Error & {
  readonly kind: DomainDataFailureKind;
  readonly status?: number;
  /** Authentication subtype, including persistence failures that can be retried. */
  readonly code?: AukiAuthFailureCode;
};

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
  billing_units: string;
  estimated_credit_cost: string;
};

export type JobEstimate = { total: string; tasks: JobEstimateTask[] };

export type JobRecord = {
  id: string;
  label: string;
  domain_id: string;
  status: JobStatus;
  priority: number;
  created_at: string;
  updated_at: string;
  organization_id: string | null;
  meta: JsonValue;
  credit_lock_id: string | null;
  credit_lock_amount: string | null;
  credit_locked_at: string | null;
  credit_released_at: string | null;
};

export type JobTaskSummary = {
  queued: number;
  leased: number;
  running: number;
  completed: number;
  failed: number;
  canceled: number;
};

export type JobListItem = { job: JobRecord; tasks_summary: JobTaskSummary };

export type JobPage = {
  items: JobListItem[];
  next_cursor: string | null;
};

export type JobTaskStatus = "queued" | "leased" | "running" | "completed" | "failed" | "canceled";

export type JobTask = {
  id: string;
  job_id: string;
  label: string;
  stage: string;
  capability: string;
  capability_filters: Record<string, string>;
  status: JobTaskStatus;
  deps_remaining: number;
  priority: number;
  inputs_cids: string[];
  outputs_prefix: string | null;
  organization_id: string | null;
  attempts: number;
  max_attempts: number;
  lease_expires_at: string | null;
  reserved_by: string | null;
  meta: JsonValue;
  cancel_requested_at: string | null;
  last_heartbeat_at: string | null;
  created_at: string;
  updated_at: string;
  mode: JobMode;
  billing_units: string;
  estimated_credit_cost: string | null;
  debited_amount: string | null;
  debited_at: string | null;
};

export type JobReceipt = {
  id: string;
  job_id: string;
  task_id: string;
  node_id: string | null;
  outputs: string[];
  meta: JsonValue;
  created_at: string;
};

export type JobDetails = {
  job: JobRecord;
  tasks_summary: JobTaskSummary;
  tasks: JobTask[];
  receipts: JobReceipt[];
};

export type JobCancellation = { id: string; status: JobStatus; updated_at: string };

export type JobsFailureKind =
  | "auth"
  | "invalid_input"
  | "invalid_response"
  | "http_status"
  | "transport"
  | "timed_out"
  | "cancelled"
  | "closed"
  | "too_large"
  | "submission_uncertain";

export type JobsError = Error & {
  readonly kind: JobsFailureKind;
  readonly status?: number;
  /** Authentication subtype, including persistence failures that can be retried. */
  readonly code?: AukiAuthFailureCode;
  /** Redacted cause category for an ambiguous submission outcome. */
  readonly source?: string;
};

export type AukiSdkExpoModuleEvents = {
  /** Internal request/ACK bridge. Contains identifiers only, never tokens. */
  onZitadelSaveRequested: (request: { sessionId: string; requestId: string }) => void;
};

/** Fleet results use the shared Rust wire names, including timestamps and nulls. */
export interface FleetQuery { capabilities?: string[]; matchAllCapabilities?: boolean }
export interface ComputePoolQuery extends FleetQuery { mode: JobMode }
export type FleetPresence = "online" | "offline" | "unknown";
export type FleetWorkState = "idle" | "busy" | "unknown";
export type FleetFailureKind = "auth" | "input" | "response" | "http" | "transport" | "timeout" | "cancelled" | "closed" | "limit";
export interface FleetError extends Error { kind: FleetFailureKind; code: string; status?: number }
export interface FleetActivity {
  worker_id: string; job_id: string; task_id: string;
  task_status: JobTaskStatus; job_status: JobStatus; mode: JobMode; capability: string;
  lease_expires_at: string | null; last_heartbeat_at: string | null;
  updated_at: string; observed_at: string;
}
export interface FleetMachine {
  kind: "robot" | "compute"; id: string; organization_id: string; name: string;
  capabilities: string[]; mode: string; association: "assigned" | "active_task" | "candidate";
  presence: FleetPresence; provider_status: string; presence_observed_at: string;
  last_seen_at: string | null; active_lease_expires_at: string | null;
  work_state: FleetWorkState; work_observed_at: string | null; activity: FleetActivity[];
}
export interface FleetSourceReport {
  source: "robots" | "nodes" | "jobs" | "busy";
  state: "complete" | "partial" | "denied" | "unsupported" | "unavailable";
  observed_at: string; codes: string[]; http_status: number | null;
}
export interface FleetSnapshot {
  domain_id: string; view: "domain" | "compute_pool"; observed_at: string;
  machines: FleetMachine[]; unresolved_activity: FleetActivity[];
  sources: FleetSourceReport[]; complete: boolean;
}
