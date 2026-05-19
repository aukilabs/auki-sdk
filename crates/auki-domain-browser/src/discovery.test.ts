import { describe, expect, it, vi } from "vitest";
import { listDomains } from "./discovery.js";

describe("listDomains", () => {
  it("maps Discovery clusters into DomainSummary rows", async () => {
    const fetcher = vi.fn().mockResolvedValue(
      new Response(
        JSON.stringify({
          clusters: [
            {
              name: "retail-lab",
              manager_peer_id: "peer-manager",
              peer_count: 2,
            },
          ],
        }),
        { status: 200 },
      ),
    );

    const result = await listDomains("http://discovery.example", fetcher);

    expect(fetcher).toHaveBeenCalledWith("http://discovery.example/clusters");
    expect(result).toEqual({
      ok: true,
      value: [{ name: "retail-lab", managerPeerId: "peer-manager", peerCount: 2 }],
    });
  });

  it("returns discovery_unreachable when fetch throws", async () => {
    const fetcher = vi.fn().mockRejectedValue(new Error("network down"));

    const result = await listDomains("http://discovery.example", fetcher);

    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.error.code).toBe("discovery_unreachable");
  });

  it("returns domain_list_failed when Discovery returns malformed JSON", async () => {
    const fetcher = vi.fn().mockResolvedValue(new Response("{", { status: 200 }));

    const result = await listDomains("http://discovery.example", fetcher);

    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.error.code).toBe("domain_list_failed");
  });

  it("returns domain_list_failed when a cluster row has no name", async () => {
    const fetcher = vi.fn().mockResolvedValue(
      new Response(JSON.stringify({ clusters: [{ peer_count: 2 }] }), { status: 200 }),
    );

    const result = await listDomains("http://discovery.example", fetcher);

    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.error.message).toContain("cluster name");
  });
});
