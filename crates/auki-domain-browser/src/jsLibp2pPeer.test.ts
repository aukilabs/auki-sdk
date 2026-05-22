import { describe, expect, it } from "vitest";
import type { PeerId } from "./contract.js";
import { createJsLibp2pBrowserPeer, type BrowserPeerTransport, type ProtocolStream } from "./jsLibp2pPeer.js";

describe("js-libp2p browser peer control plane", () => {
  it("serves and fetches remote info and sensors over native Auki protocols", async () => {
    const network = new MemoryNetwork();
    const transportA = network.createTransport("peer-a", ["/memory/peer-a"]);
    const transportB = network.createTransport("peer-b", ["/memory/peer-b"]);
    network.setJoinMembership({
      managerPeerId: "manager-peer",
      membershipJson: JSON.stringify({
        cluster_name: "demo",
        peers: [
          { peer_id: "peer-a", multiaddrs: ["/memory/peer-a"] },
          { peer_id: "peer-b", multiaddrs: ["/memory/peer-b"] },
        ],
      }),
    });

    const peerA = await createJsLibp2pBrowserPeer({
      peerId: "peer-a",
      transport: transportA,
      resolveJoinTarget: async () => ({
        domainName: "demo",
        managerPeerId: "manager-peer",
        managerMultiaddrs: ["/memory/manager"],
      }),
    });
    const peerB = await createJsLibp2pBrowserPeer({
      peerId: "peer-b",
      transport: transportB,
      resolveJoinTarget: async () => ({
        domainName: "demo",
        managerPeerId: "manager-peer",
        managerMultiaddrs: ["/memory/manager"],
      }),
    });

    await peerB.setParticipantMetadata({ appId: "park", displayName: "Browser B" });
    await peerB.declareLocalSensors([
      {
        id: "audio",
        kind: "audio",
        label: "Microphone",
        publishable: true,
        subscribable: true,
      },
    ]);

    const snapshots: unknown[] = [];
    peerA.observeParticipants((snapshot) => snapshots.push(snapshot));

    await expect(peerB.joinDomain("inline-manager://ignored", "demo")).resolves.toEqual({
      ok: true,
      value: undefined,
    });
    await expect(peerA.joinDomain("inline-manager://ignored", "demo")).resolves.toEqual({
      ok: true,
      value: undefined,
    });

    expect(network.openedProtocols).toContain("/auki/info/0.0.1");
    expect(network.openedProtocols).toContain("/auki/sensors/0.0.1");
    expect(snapshots.at(-1)).toMatchObject({
      participants: expect.arrayContaining([
        expect.objectContaining({
          peerId: "peer-b",
          appId: "park",
          displayName: "Browser B",
          sensors: [
            {
              id: "audio",
              kind: "audio",
              label: "audio",
              publishable: true,
              subscribable: true,
            },
          ],
        }),
      ]),
    });
  });

  it("subscribes to remote browser audio over /auki/stream/0.1.0", async () => {
    const network = new MemoryNetwork();
    const transportA = network.createTransport("peer-a", ["/memory/peer-a"]);
    const transportB = network.createTransport("peer-b", ["/memory/peer-b"]);
    network.setJoinMembership({
      managerPeerId: "manager-peer",
      membershipJson: JSON.stringify({
        cluster_name: "demo",
        peers: [
          { peer_id: "peer-a", multiaddrs: ["/memory/peer-a"] },
          { peer_id: "peer-b", multiaddrs: ["/memory/peer-b"] },
        ],
      }),
    });
    const peerA = await createJsLibp2pBrowserPeer({
      peerId: "peer-a",
      transport: transportA,
      resolveJoinTarget: async () => ({
        domainName: "demo",
        managerPeerId: "manager-peer",
        managerMultiaddrs: ["/memory/manager"],
      }),
    });
    const peerB = await createJsLibp2pBrowserPeer({
      peerId: "peer-b",
      transport: transportB,
      resolveJoinTarget: async () => ({
        domainName: "demo",
        managerPeerId: "manager-peer",
        managerMultiaddrs: ["/memory/manager"],
      }),
    });
    await peerB.declareLocalSensors([
      {
        id: "audio",
        kind: "audio",
        label: "Microphone",
        publishable: true,
        subscribable: true,
      },
    ]);
    await peerB.setSensorPublication("audio", true);
    const snapshots: unknown[] = [];
    peerA.observeParticipants((snapshot) => snapshots.push(snapshot));

    await peerB.joinDomain("inline-manager://ignored", "demo");
    await peerA.joinDomain("inline-manager://ignored", "demo");
    await expect(peerA.subscribeToSensor("peer-b", "audio")).resolves.toEqual({
      ok: true,
      value: undefined,
    });

    expect(network.openedProtocols).toContain("/auki/stream/0.1.0");
    expect(snapshots.at(-1)).toMatchObject({
      participants: expect.arrayContaining([
        expect.objectContaining({
          peerId: "peer-a",
          mediaPresence: expect.objectContaining({
            listeningToPeerId: "peer-b",
            listeningToSensorId: "audio",
            playbackHealthy: true,
            selectedRemoteStreamState: "connected",
            lastFrameUnixMs: expect.any(Number),
            outputLevel: expect.any(Number),
          }),
        }),
      ]),
    });
  });

  it("retries transient audio stream open failures", async () => {
    const network = new MemoryNetwork();
    const transportA = network.createTransport("peer-a", ["/memory/peer-a"]);
    const transportB = network.createTransport("peer-b", ["/memory/peer-b"]);
    network.setJoinMembership({
      managerPeerId: "manager-peer",
      membershipJson: JSON.stringify({
        cluster_name: "demo",
        peers: [
          { peer_id: "peer-a", multiaddrs: ["/memory/peer-a"] },
          { peer_id: "peer-b", multiaddrs: ["/memory/peer-b"] },
        ],
      }),
    });
    const peerA = await createJsLibp2pBrowserPeer({
      peerId: "peer-a",
      transport: transportA,
      resolveJoinTarget: async () => ({
        domainName: "demo",
        managerPeerId: "manager-peer",
        managerMultiaddrs: ["/memory/manager"],
      }),
    });
    const peerB = await createJsLibp2pBrowserPeer({
      peerId: "peer-b",
      transport: transportB,
      resolveJoinTarget: async () => ({
        domainName: "demo",
        managerPeerId: "manager-peer",
        managerMultiaddrs: ["/memory/manager"],
      }),
    });
    await peerB.declareLocalSensors([
      {
        id: "audio",
        kind: "audio",
        label: "Microphone",
        publishable: true,
        subscribable: true,
      },
    ]);
    await peerB.setSensorPublication("audio", true);
    network.failNextDial("peer-a", "peer-b", "/auki/stream/0.1.0", "Remote closed connection during opening");

    await peerB.joinDomain("inline-manager://ignored", "demo");
    await peerA.joinDomain("inline-manager://ignored", "demo");

    await expect(peerA.subscribeToSensor("peer-b", "audio")).resolves.toEqual({
      ok: true,
      value: undefined,
    });
    expect(network.openedProtocols.filter((protocol) => protocol === "/auki/stream/0.1.0")).toHaveLength(2);
  });
});

class MemoryNetwork {
  readonly transports = new Map<PeerId, MemoryTransport>();
  readonly openedProtocols: string[] = [];
  private dialFailures = new Map<string, Error>();
  private managerPeerId = "manager-peer";
  private membershipJson = JSON.stringify({ cluster_name: "demo", peers: [] });

  createTransport(peerId: PeerId, advertisedMultiaddrs: string[]): MemoryTransport {
    const transport = new MemoryTransport(this, peerId, advertisedMultiaddrs);
    this.transports.set(peerId, transport);
    return transport;
  }

  setJoinMembership(next: { managerPeerId: PeerId; membershipJson: string }): void {
    this.managerPeerId = next.managerPeerId;
    this.membershipJson = next.membershipJson;
  }

  failNextDial(from: PeerId, to: PeerId, protocol: string, message: string): void {
    this.dialFailures.set(`${from}\0${to}\0${protocol}`, new Error(message));
  }

  async dialProtocol(from: PeerId, to: PeerId, protocol: string): Promise<ProtocolStream> {
    this.openedProtocols.push(protocol);
    const failureKey = `${from}\0${to}\0${protocol}`;
    const failure = this.dialFailures.get(failureKey);
    if (failure) {
      this.dialFailures.delete(failureKey);
      throw failure;
    }
    if (to === this.managerPeerId && protocol === "/auki/join/0.0.1") {
      const [client, server] = linkedStreams();
      void import("./protocol/control.js").then(async ({ JoinRequest, JoinResponse, readFrame, writeFrame }) => {
        await readFrame(server, JoinRequest);
        await writeFrame(
          server,
          JoinResponse,
          {
            kind: {
              case: "accept",
              value: {
                membershipJson: this.membershipJson,
                successorToken: new Uint8Array(),
              },
            },
          },
        );
        await server.close();
      });
      return client;
    }

    const remote = this.transports.get(to);
    const handler = remote?.handlers.get(protocol);
    if (!handler) {
      throw new Error(`no handler for ${to} ${protocol}`);
    }
    const [client, server] = linkedStreams();
    void handler(server, from);
    return client;
  }
}

class MemoryTransport implements BrowserPeerTransport {
  readonly handlers = new Map<string, (stream: ProtocolStream, remotePeerId: PeerId) => Promise<void>>();

  constructor(
    private readonly network: MemoryNetwork,
    readonly peerId: PeerId,
    private readonly addrs: string[],
  ) {}

  async start(): Promise<void> {}

  async stop(): Promise<void> {}

  advertisedMultiaddrs(): string[] {
    return this.addrs;
  }

  async handleProtocol(
    protocol: string,
    handler: (stream: ProtocolStream, remotePeerId: PeerId) => Promise<void>,
  ): Promise<void> {
    this.handlers.set(protocol, handler);
  }

  async dialProtocol(peerId: PeerId, _multiaddrs: string[], protocol: string): Promise<ProtocolStream> {
    return this.network.dialProtocol(this.peerId, peerId, protocol);
  }
}

class QueueStream implements ProtocolStream {
  private peer: QueueStream | null = null;
  private chunks: Uint8Array[] = [];
  private waiters: Array<() => void> = [];
  private closed = false;

  connect(peer: QueueStream): void {
    this.peer = peer;
  }

  send(data: Uint8Array): boolean {
    this.peer?.push(data);
    return true;
  }

  async close(): Promise<void> {
    this.peer?.finish();
    this.finish();
  }

  async onDrain(): Promise<void> {}

  async *[Symbol.asyncIterator](): AsyncIterableIterator<Uint8Array> {
    while (!this.closed || this.chunks.length > 0) {
      const chunk = this.chunks.shift();
      if (chunk) {
        yield chunk;
        continue;
      }
      await new Promise<void>((resolve) => this.waiters.push(resolve));
    }
  }

  private push(chunk: Uint8Array): void {
    this.chunks.push(chunk);
    this.wake();
  }

  private finish(): void {
    this.closed = true;
    this.wake();
  }

  private wake(): void {
    const waiters = this.waiters.splice(0);
    for (const waiter of waiters) waiter();
  }
}

function linkedStreams(): [QueueStream, QueueStream] {
  const a = new QueueStream();
  const b = new QueueStream();
  a.connect(b);
  b.connect(a);
  return [a, b];
}
