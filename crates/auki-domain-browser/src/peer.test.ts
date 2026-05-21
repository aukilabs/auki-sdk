import { afterEach, describe, expect, it, vi } from "vitest";
import { createBrowserDomainPeer } from "./peer.js";

describe("createBrowserDomainPeer", () => {
  const installedFactory = window.aukiBrowserPeer;

  afterEach(() => {
    if (installedFactory) {
      window.aukiBrowserPeer = { createPeer: installedFactory.createPeer };
    } else {
      delete window.aukiBrowserPeer;
    }
  });

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

  it("uses a global factory when browser transport is injected", async () => {
    const session = {
      getSelfPeerId: vi.fn().mockResolvedValue("global-peer"),
      listDomains: vi.fn().mockResolvedValue({ ok: true, value: [] }),
      createDomain: vi.fn().mockResolvedValue({ ok: true, value: undefined }),
      joinDomain: vi.fn().mockResolvedValue({ ok: true, value: undefined }),
      leaveDomain: vi.fn().mockResolvedValue({ ok: true, value: undefined }),
      observeParticipants: vi.fn(),
      setParticipantMetadata: vi.fn().mockResolvedValue({ ok: true, value: undefined }),
      declareLocalSensors: vi.fn().mockResolvedValue({ ok: true, value: undefined }),
      setSensorPublication: vi.fn().mockResolvedValue({ ok: true, value: undefined }),
      subscribeToSensor: vi.fn().mockResolvedValue({ ok: true, value: undefined }),
      unsubscribeFromSensor: vi.fn().mockResolvedValue({ ok: true, value: undefined }),
    };

    window.aukiBrowserPeer = { createPeer: vi.fn(() => Promise.resolve(session)) };

    const peer = await createBrowserDomainPeer({ peerId: "self-peer" });
    const result = await peer.joinDomain("http://discovery.example", "demo");

    expect(window.aukiBrowserPeer?.createPeer).toHaveBeenCalledTimes(1);
    expect(result).toEqual({ ok: true, value: undefined });
    expect(session.joinDomain).toHaveBeenCalledWith("http://discovery.example", "demo");
  });

  it("delegates SDK methods when a session is injected", async () => {
    const session = {
      getSelfPeerId: vi.fn().mockResolvedValue("session-peer"),
      listDomains: vi.fn().mockResolvedValue({ ok: true, value: [] }),
      createDomain: vi.fn().mockResolvedValue({ ok: true, value: undefined }),
      joinDomain: vi.fn().mockResolvedValue({ ok: true, value: undefined }),
      leaveDomain: vi.fn().mockResolvedValue({ ok: true, value: undefined }),
      observeParticipants: vi.fn(),
      setParticipantMetadata: vi.fn().mockResolvedValue({ ok: true, value: undefined }),
      declareLocalSensors: vi.fn().mockResolvedValue({ ok: true, value: undefined }),
      setSensorPublication: vi.fn().mockResolvedValue({ ok: true, value: undefined }),
      subscribeToSensor: vi.fn().mockResolvedValue({ ok: true, value: undefined }),
      unsubscribeFromSensor: vi.fn().mockResolvedValue({ ok: true, value: undefined }),
    };

    const peer = await createBrowserDomainPeer({ peerId: "self-peer", sdkSession: session });
    const result = await peer.createDomain("http://discovery.example", "demo");

    expect(result).toEqual({ ok: true, value: undefined });
    expect(session.createDomain).toHaveBeenCalledWith("http://discovery.example", "demo");
  });

  it("wraps a wasm SDK session that exposes peerId and transport methods", async () => {
    const session = {
      peerId: vi.fn(() => "wasm-peer"),
      createDomain: vi.fn().mockReturnValue({ ok: false, error: { code: "transport_unavailable", message: "transport pending" } }),
      joinDomain: vi.fn().mockReturnValue({ ok: false, error: { code: "transport_unavailable", message: "transport pending" } }),
      leaveDomain: vi.fn().mockReturnValue({ ok: true, value: undefined }),
      setSensorPublication: vi.fn().mockReturnValue({ ok: false, error: { code: "transport_unavailable", message: "transport pending" } }),
      subscribeToSensor: vi.fn().mockReturnValue({ ok: false, error: { code: "transport_unavailable", message: "transport pending" } }),
      unsubscribeFromSensor: vi.fn().mockReturnValue({ ok: false, error: { code: "transport_unavailable", message: "transport pending" } }),
    };

    const peer = await createBrowserDomainPeer({ peerId: "fallback-peer", sdkSession: session });

    await expect(peer.getSelfPeerId()).resolves.toBe("wasm-peer");
    await expect(peer.createDomain("http://discovery.example", "demo")).resolves.toEqual({
      ok: false,
      error: { code: "transport_unavailable", message: "transport pending" },
    });
    expect(session.createDomain).toHaveBeenCalledWith("http://discovery.example", "demo");
  });

  it("emits a joined snapshot from wasm join metadata", async () => {
    const membershipJson = JSON.stringify({
      cluster_name: "demo",
      peers: [
        {
          peer_id: "wasm-peer",
          multiaddrs: [],
          join_ts_ns: 1,
          successor_token: [],
        },
      ],
    });
    const session = {
      peerId: vi.fn(() => "wasm-peer"),
      joinDomain: vi.fn().mockResolvedValue({
        ok: true,
        value: {
          domainName: "demo",
          managerPeerId: "manager-peer",
          membershipJson,
        },
      }),
      leaveDomain: vi.fn().mockReturnValue({ ok: true, value: undefined }),
    };

    const peer = await createBrowserDomainPeer({ peerId: "fallback-peer", sdkSession: session });
    const snapshots: unknown[] = [];
    peer.observeParticipants((snapshot) => snapshots.push(snapshot));
    await peer.setParticipantMetadata({ appId: "park", displayName: "Operator" });
    await peer.declareLocalSensors([
      {
        id: "audio",
        kind: "audio",
        label: "Microphone",
        publishable: true,
        subscribable: false,
      },
    ]);

    const result = await peer.joinDomain("http://discovery.example", "demo");

    expect(result).toEqual({ ok: true, value: undefined });
    expect(snapshots.at(-1)).toMatchObject({
      selfPeerId: "wasm-peer",
      domainName: "demo",
      managerPeerId: "manager-peer",
      electionState: "stable",
      participants: [
        {
          peerId: "wasm-peer",
          appId: "park",
          displayName: "Operator",
          isSelf: true,
          connected: true,
          sensors: [
            {
              id: "audio",
              kind: "audio",
              label: "Microphone",
              publishable: true,
              subscribable: false,
            },
          ],
        },
        {
          peerId: "manager-peer",
          isSelf: false,
          connected: true,
        },
      ],
    });
  });

  it("updates fallback media presence when SDK media operations succeed", async () => {
    const session = {
      peerId: vi.fn(() => "wasm-peer"),
      joinDomain: vi.fn().mockResolvedValue({
        ok: true,
        value: {
          domainName: "demo",
          managerPeerId: "manager-peer",
        },
      }),
      setSensorPublication: vi.fn().mockResolvedValue({ ok: true, value: undefined }),
      subscribeToSensor: vi.fn().mockResolvedValue({ ok: true, value: undefined }),
      unsubscribeFromSensor: vi.fn().mockResolvedValue({ ok: true, value: undefined }),
    };

    const peer = await createBrowserDomainPeer({ peerId: "fallback-peer", sdkSession: session });
    const snapshots: unknown[] = [];
    peer.observeParticipants((snapshot) => snapshots.push(snapshot));

    await peer.joinDomain("http://discovery.example", "demo");
    await peer.setSensorPublication("audio", true);
    await peer.subscribeToSensor("remote-peer", "audio");

    expect(snapshots.at(-1)).toMatchObject({
      participants: expect.arrayContaining([
        expect.objectContaining({
          peerId: "wasm-peer",
          mediaPresence: expect.objectContaining({
            micAvailable: true,
            micPublicationEnabled: true,
            micCaptureHealthy: true,
            listeningToPeerId: "remote-peer",
            listeningToSensorId: "audio",
            selectedRemoteStreamState: "connecting",
          }),
        }),
      ]),
    });

    await peer.unsubscribeFromSensor("remote-peer", "audio");

    expect(snapshots.at(-1)).toMatchObject({
      participants: expect.arrayContaining([
        expect.objectContaining({
          peerId: "wasm-peer",
          mediaPresence: expect.objectContaining({
            listeningToPeerId: null,
            listeningToSensorId: null,
            selectedRemoteStreamState: "off",
          }),
        }),
      ]),
    });
  });

  it("falls back to transport-closed when global factory fails", async () => {
    window.aukiBrowserPeer = {
      createPeer: () => Promise.reject(new Error("browser sdk not ready")),
    };

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

  it("fails closed for all transport-backed operations until browser transport exists", async () => {
    const peer = await createBrowserDomainPeer({ peerId: "self-peer" });
    const operations = [
      () => peer.createDomain("http://discovery.example", "demo"),
      () => peer.joinDomain("http://discovery.example", "demo"),
      () => peer.setSensorPublication("mic", true),
      () => peer.subscribeToSensor("remote-peer", "mic"),
      () => peer.unsubscribeFromSensor("remote-peer", "mic"),
    ];

    for (const operation of operations) {
      await expect(operation()).resolves.toEqual({
        ok: false,
        error: {
          code: "transport_unavailable",
          message: "Browser SDK transport is not implemented yet.",
        },
      });
    }
  });
});
