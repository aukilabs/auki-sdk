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

export type AukiSdkExpoModuleEvents = {
  /** Internal request/ACK bridge. Contains identifiers only, never tokens. */
  onZitadelSaveRequested: (request: { sessionId: string; requestId: string }) => void;
};
