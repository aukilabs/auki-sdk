import { sdkRuntime, type DiscoveryClusterEntry } from "../sdk/runtime";

export type { DiscoveryClusterEntry };

export type DiscoverySnapshot = {
  discovery_url: string;
  fetched_at_unix_ms: number;
  clusters: DiscoveryClusterEntry[];
  raw_json: string;
};

export async function fetchDiscoverySnapshot(
  discoveryUrl?: string,
): Promise<DiscoverySnapshot> {
  const url = discoveryUrl?.trim() ?? "";
  const clusters = await sdkRuntime.listClusters(url);
  return {
    discovery_url: url,
    fetched_at_unix_ms: Date.now(),
    clusters,
    raw_json: JSON.stringify({ clusters }),
  };
}
