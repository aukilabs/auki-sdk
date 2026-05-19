import type { DomainSummary, Result } from "./contract";
import { fail, ok } from "./errors";

type Fetcher = (url: string) => Promise<Response>;

type DiscoveryCluster = {
  name: string;
  manager_peer_id?: string;
  peer_count?: number;
};

export async function listDomains(
  discoveryUrl: string,
  fetcher: Fetcher = fetch,
): Promise<Result<DomainSummary[]>> {
  const base = discoveryUrl.replace(/\/+$/, "");
  try {
    const response = await fetcher(`${base}/clusters`);
    if (!response.ok) {
      return fail("domain_list_failed", `Discovery returned HTTP ${response.status}`);
    }
    const body = (await response.json()) as { clusters?: DiscoveryCluster[] };
    const clusters = Array.isArray(body.clusters) ? body.clusters : [];
    return ok(
      clusters.map((cluster) => ({
        name: cluster.name,
        managerPeerId: cluster.manager_peer_id,
        peerCount: cluster.peer_count,
      })),
    );
  } catch (error) {
    const detail = error instanceof Error ? error.message : "unknown fetch error";
    return fail("discovery_unreachable", `Discovery unreachable: ${detail}`);
  }
}
