import { readFile } from "node:fs/promises";
import path from "node:path";
import { describe, expect, it } from "vitest";
import { peerIdFromSeed } from "./identity.js";
import { createAukiBrowserPeer, type SpatialMessage } from "./peer.js";
import { JsonFrameReader, writeJsonFrame } from "./stream.js";
import { createPeerBinding, createPeerHandshake, type JsonObject } from "./protocol.js";
import type { BrowserProtocolStream, BrowserTransport } from "./transport.js";

describe("AukiBrowserPeer shell", () => {
  it("connects bootstrap records through an injected transport", async () => {
    const transport = new MemoryTransport("browser-peer", ["/p2p-circuit/p2p/browser-peer"]);
    const peer = await createAukiBrowserPeer({
      transport,
      bootstrap: bootstrapRecord("native-peer", "/memory/native-direct"),
      protocolWasm: await protocolWasmInput(),
    });

    expect(peer.peerId).toBe("browser-peer");
    expect(peer.supportedTransports).toContain("webrtc_direct");
    expect(peer.listPeers()).toEqual([
      {
        peerId: "native-peer",
        connected: false,
        dialAddresses: ["/memory/native-direct"],
      },
    ]);

    await peer.connectBootstrap([
      bootstrapRecord("native-peer", "/memory/native-direct"),
      bootstrapRecord("relay-peer", "/memory/relay"),
    ]);

    expect(transport.started).toBe(1);
    expect(transport.dials).toEqual([["/memory/native-direct"], ["/memory/relay"]]);
    expect(peer.listPeers()).toEqual([
      {
        peerId: "native-peer",
        connected: true,
        dialAddresses: ["/memory/native-direct"],
      },
      {
        peerId: "relay-peer",
        connected: true,
        dialAddresses: ["/memory/relay"],
      },
    ]);
  });

  it("stops the underlying transport", async () => {
    const transport = new MemoryTransport("browser-peer", []);
    const peer = await createAukiBrowserPeer({ transport, protocolWasm: await protocolWasmInput() });

    await peer.dial("/memory/native");
    await peer.stop();

    expect(transport.started).toBe(1);
    expect(transport.stopped).toBe(1);
  });

  it("exchanges lifecycle handshakes when the browser has a seed", async () => {
    const localSeed = new Uint8Array(32).fill(7);
    const remoteSeed = new Uint8Array(32).fill(9);
    const localPeerId = await peerIdFromSeed(localSeed);
    const remotePeerId = await peerIdFromSeed(remoteSeed);
    const transport = new MemoryTransport(localPeerId, []);
    transport.handleProtocol(LIFECYCLE_PROTOCOL_ID, async (stream) => {
      const reader = new JsonFrameReader(stream);
      const localHandshake = await reader.read(DEFAULT_FRAME_BODY_LIMIT);
      expect(localHandshake.value.type).toBe("auki.peer_handshake.v1");

      const remoteBinding = await createPeerBinding(
        remoteSeed,
        remotePeerId,
        new Date(Date.now() - 1_000).toISOString(),
        "native-peer",
      );
      await writeJsonFrame(
        stream,
        await createPeerHandshake(remoteBinding),
        DEFAULT_FRAME_BODY_LIMIT,
      );
      await stream.close();
    });

    const peer = await createAukiBrowserPeer({
      seed: localSeed,
      transport,
      protocolWasm: await protocolWasmInput(),
    });

    await peer.connectBootstrap(bootstrapRecord(remotePeerId, "/memory/native-direct"));

    expect(transport.protocolDials).toEqual([
      {
        peerId: remotePeerId,
        addresses: ["/memory/native-direct"],
        protocol: LIFECYCLE_PROTOCOL_ID,
      },
    ]);
  });

  it("loads offer catalogs over an RFC protocol stream", async () => {
    const fixture = await fixtureJson("v1_offer_catalogs.json");
    const catalog = fixture.positive.response_with_offer.object as JsonObject;
    const transport = new MemoryTransport("browser-peer", []);
    transport.handleProtocol(OFFER_CATALOG_PROTOCOL_ID, async (stream) => {
      const reader = new JsonFrameReader(stream);
      const request = await reader.read(DEFAULT_FRAME_BODY_LIMIT);
      expect(request.value).toEqual({
        type: "auki.offer_catalog_request.v1",
      });
      await writeJsonFrame(stream, catalog, DEFAULT_FRAME_BODY_LIMIT);
      await stream.close();
    });

    const peer = await createAukiBrowserPeer({
      transport,
      bootstrap: bootstrapRecord("native-peer", "/memory/native-direct"),
      protocolWasm: await protocolWasmInput(),
    });

    await expect(peer.listOffers("native-peer")).resolves.toEqual([
      {
        peerId: "native-peer",
        domainId: fixture.inputs.domain_id,
        offerId: "camera-main",
        kind: "sensor.frame",
        payloadType: "auki.frame",
        accessModes: ["get", "subscribe"],
      },
    ]);
    expect(transport.protocolDials).toEqual([
      {
        peerId: "native-peer",
        addresses: ["/memory/native-direct"],
        protocol: OFFER_CATALOG_PROTOCOL_ID,
      },
    ]);
  });

  it("subscribes to spatial messages over an RFC protocol stream", async () => {
    const offerFixture = await fixtureJson("v1_offer_catalogs.json");
    const subscribeFixture = await fixtureJson("v1_subscribe.json");
    const catalog = offerFixture.positive.response_with_offer.object as JsonObject;
    const accept = subscribeFixture.positive.accept_start_result.object as JsonObject;
    const data = subscribeFixture.positive.data_message.object as JsonObject;
    const end = subscribeFixture.positive.end_message.object as JsonObject;
    const inputs = subscribeFixture.inputs as JsonObject;
    const transport = new MemoryTransport("browser-peer", []);
    transport.handleProtocol(OFFER_CATALOG_PROTOCOL_ID, async (stream) => {
      const reader = new JsonFrameReader(stream);
      await reader.read(DEFAULT_FRAME_BODY_LIMIT);
      await writeJsonFrame(stream, catalog, DEFAULT_FRAME_BODY_LIMIT);
      await stream.close();
    });
    transport.handleProtocol(SUBSCRIBE_PROTOCOL_ID, async (stream) => {
      const reader = new JsonFrameReader(stream);
      const request = await reader.read(DEFAULT_FRAME_BODY_LIMIT);
      expect(request.value).toEqual(subscribeFixture.positive.request.object);
      await writeJsonFrame(stream, accept, DEFAULT_FRAME_BODY_LIMIT);
      await writeJsonFrame(stream, data, DEFAULT_FRAME_BODY_LIMIT);
      await writeJsonFrame(stream, end, DEFAULT_FRAME_BODY_LIMIT);
      await stream.close();
    });

    const peer = await createAukiBrowserPeer({
      transport,
      bootstrap: bootstrapRecord("native-peer", "/memory/native-direct"),
      protocolWasm: await protocolWasmInput(),
    });

    const messages: SpatialMessage[] = [];
    for await (const message of peer.subscribe({
      peerId: "native-peer",
      domainId: inputs.domain_id as string,
      offerId: inputs.offer_id as string,
      params: { frame: "latest", stream: "live" },
      acceptedPayloadTypes: [inputs.selected_payload_type as string],
      maxMessageBytes: inputs.max_message_bytes as number,
    })) {
      messages.push(message);
    }

    expect(messages).toEqual([data]);
    expect(transport.protocolDials.map((dial) => dial.protocol)).toEqual([
      OFFER_CATALOG_PROTOCOL_ID,
      SUBSCRIBE_PROTOCOL_ID,
    ]);
  });
});

const LIFECYCLE_PROTOCOL_ID = "/auki/cluster-lifecycle/0.0.1";
const OFFER_CATALOG_PROTOCOL_ID = "/auki/offer-catalog/0.0.1";
const SUBSCRIBE_PROTOCOL_ID = "/auki/subscribe/0.0.1";
const DEFAULT_FRAME_BODY_LIMIT = 1_048_576;

function bootstrapRecord(peerId: string, address: string): unknown {
  return {
    peer_id: peerId,
    direct_addresses: [address],
    webrtc_direct_addresses: [],
    relay_addresses: [],
    relay_server_addresses: [],
    bootstrap_addresses: [address],
  };
}

async function protocolWasmInput(): Promise<{ module_or_path: Uint8Array }> {
  return {
    module_or_path: await readFile(
      path.resolve(process.cwd(), "../auki-protocol-wasm/pkg-web/auki_protocol_wasm_bg.wasm"),
    ),
  };
}

async function fixtureJson(name: string): Promise<JsonObject> {
  const content = await readFile(
    path.resolve(process.cwd(), "../auki-protocol/vectors", name),
    "utf8",
  );
  return JSON.parse(content) as JsonObject;
}

class MemoryTransport implements BrowserTransport {
  readonly dials: string[][] = [];
  readonly protocolDials: Array<{ peerId: string; addresses: string[]; protocol: string }> = [];
  private readonly protocolHandlers = new Map<
    string,
    (stream: BrowserProtocolStream, peerId: string) => Promise<void> | void
  >();
  started = 0;
  stopped = 0;

  constructor(
    readonly peerId: string,
    private readonly addresses: string[],
  ) {}

  async start(): Promise<void> {
    this.started += 1;
  }

  async stop(): Promise<void> {
    this.stopped += 1;
  }

  multiaddrs(): string[] {
    return this.addresses.slice();
  }

  async dial(addresses: string[]): Promise<void> {
    this.dials.push(addresses.slice());
  }

  handleProtocol(
    protocol: string,
    handler: (stream: BrowserProtocolStream, peerId: string) => Promise<void> | void,
  ): void {
    this.protocolHandlers.set(protocol, handler);
  }

  async dialProtocol(
    peerId: string,
    addresses: string[],
    protocol: string,
  ): Promise<BrowserProtocolStream> {
    this.protocolDials.push({ peerId, addresses: addresses.slice(), protocol });
    const handler = this.protocolHandlers.get(protocol);
    if (!handler) {
      throw new Error(`No handler registered for ${protocol}`);
    }
    const [local, remote] = linkedStreams();
    Promise.resolve(handler(remote, peerId)).catch((error: unknown) => {
      remote.abort(error instanceof Error ? error : new Error(String(error)));
    });
    return local;
  }
}

function linkedStreams(): [QueueStream, QueueStream] {
  const left = new QueueStream();
  const right = new QueueStream();
  left.link(right);
  right.link(left);
  return [left, right];
}

class QueueStream implements BrowserProtocolStream {
  private peer?: QueueStream;
  private readonly chunks: Uint8Array[] = [];
  private readonly waiters: Array<() => void> = [];
  private closed = false;
  private aborted?: Error;

  link(peer: QueueStream): void {
    this.peer = peer;
  }

  send(data: Uint8Array): boolean {
    if (this.closed || this.aborted) {
      return false;
    }
    this.peer?.push(data);
    return true;
  }

  async close(): Promise<void> {
    this.finish();
    this.peer?.finish();
  }

  abort(error: Error): void {
    this.fail(error);
    this.peer?.fail(error);
  }

  async onDrain(): Promise<void> {}

  async *[Symbol.asyncIterator](): AsyncIterator<Uint8Array> {
    for (;;) {
      const chunk = this.chunks.shift();
      if (chunk) {
        yield chunk;
        continue;
      }
      if (this.aborted) {
        throw this.aborted;
      }
      if (this.closed) {
        return;
      }
      await new Promise<void>((resolve) => this.waiters.push(resolve));
    }
  }

  private push(data: Uint8Array): void {
    if (this.closed || this.aborted) {
      return;
    }
    this.chunks.push(data.slice());
    this.wake();
  }

  private finish(): void {
    this.closed = true;
    this.wake();
  }

  private fail(error: Error): void {
    this.aborted = error;
    this.closed = true;
    this.wake();
  }

  private wake(): void {
    for (const resolve of this.waiters.splice(0)) {
      resolve();
    }
  }
}
