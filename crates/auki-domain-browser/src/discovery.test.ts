import { describe, expect, it, vi } from "vitest";
import { listDomains } from "./discovery";

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
});
