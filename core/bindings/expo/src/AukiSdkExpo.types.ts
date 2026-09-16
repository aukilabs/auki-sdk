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

export type AukiSdkExpoModuleEvents = {
  /** Internal request/ACK bridge. Contains identifiers only, never tokens. */
  onZitadelSaveRequested: (request: { sessionId: string; requestId: string }) => void;
};
