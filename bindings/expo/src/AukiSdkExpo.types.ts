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

export type AukiSdkExpoModuleEvents = Record<string, never>;
