import type { DomainSummary, Result } from "./contract.js";
import { fail, ok } from "./errors.js";

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
  let response: Response;
  try {
    response = await fetcher(`${base}/clusters`);
  } catch (error) {
    const detail = error instanceof Error ? error.message : "unknown fetch error";
    return fail("discovery_unreachable", `Discovery unreachable: ${detail}`);
  }

  if (!response.ok) {
    return fail("domain_list_failed", `Discovery returned HTTP ${response.status}`);
  }

  let body: { clusters?: DiscoveryCluster[] };
  try {
    body = (await response.json()) as { clusters?: DiscoveryCluster[] };
  } catch (error) {
    const detail = error instanceof Error ? error.message : "unknown JSON error";
    return fail("domain_list_failed", `Discovery returned malformed JSON: ${detail}`);
  }

  const clusters = Array.isArray(body.clusters) ? body.clusters : [];
  const domains: DomainSummary[] = [];
  for (const cluster of clusters) {
    if (typeof cluster.name !== "string" || cluster.name.length === 0) {
      return fail("domain_list_failed", "Discovery cluster row is missing a cluster name.");
    }
    domains.push({
      name: cluster.name,
      managerPeerId:
        typeof cluster.manager_peer_id === "string" ? cluster.manager_peer_id : undefined,
      peerCount: typeof cluster.peer_count === "number" ? cluster.peer_count : undefined,
    });
  }

  return ok(domains);
}
