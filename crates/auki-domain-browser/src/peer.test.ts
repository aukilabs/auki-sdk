import { describe, expect, it, vi } from "vitest";
import { createBrowserDomainPeer } from "./peer";

describe("createBrowserDomainPeer", () => {
  it("emits an idle unjoined snapshot immediately", async () => {
    const peer = await createBrowserDomainPeer({ peerId: "self-peer" });
    const snapshots: unknown[] = [];

    peer.observeParticipants((snapshot) => snapshots.push(snapshot));

    expect(snapshots).toEqual([
      {
        selfPeerId: "self-peer",
        domainName: null,
        participants: [],
        managerPeerId: null,
        electionState: "unknown",
      },
    ]);
  });

  it("delegates listDomains to Discovery mapping", async () => {
    const fetcher = vi.fn().mockResolvedValue(
      new Response(JSON.stringify({ clusters: [{ name: "demo" }] }), { status: 200 }),
    );
    const peer = await createBrowserDomainPeer({ peerId: "self-peer", fetcher });

    const result = await peer.listDomains("http://discovery.example");

    expect(result).toEqual({ ok: true, value: [{ name: "demo" }] });
  });

  it("fails closed for join until browser transport exists", async () => {
    const peer = await createBrowserDomainPeer({ peerId: "self-peer" });

    const result = await peer.joinDomain("http://discovery.example", "demo");

    expect(result).toEqual({
      ok: false,
      error: {
        code: "transport_unavailable",
        message: "Browser SDK transport is not implemented yet.",
      },
    });
  });
});
