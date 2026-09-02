import {
  AukiBlobClient,
  AukiBlobEndpoint,
  AukiCatalogClient,
  AukiCatalogEndpoint,
  AukiDiscoveryMode,
  AukiInfoClient,
  AukiInfoEndpoint,
  AukiMessageClient,
  AukiMessageEndpoint,
  AukiPeer,
  AukiRegistryClient,
  AukiRegistryEndpoint,
  AukiStreamClient,
  AukiStreamEndpoint,
  type AukiAuthenticatedPeer,
  type AukiBlobProviderRequest,
  type AukiCatalogResource,
  type AukiCatalogResourcesRequest,
  type AukiExactTarget,
  type AukiMessageChannelResource,
  type AukiRegistryProviderRequest,
  type AukiStreamManifest,
  type AukiStreamRequest,
  type AukiStreamSourceItem,
} from "../pkg-web/auki_sdk_web.js";

const APP = "standard-protocols";
const APP_VERSION = "0.1.0";
const BLOB_BYTES = new TextEncoder().encode("auki-standard-protocols-v1");
const BLOB_SHA256 = "bc170af4cf7bb5266683f459f5121348f60a7a5ee7d35a9bf7f5d29fe8fa3b96";
const MESSAGE_RESOURCE_ID = "playground/events";
const MESSAGE_CLOCK_ID = "playground/clock";
const MESSAGE_CLOCK_HASH = "playground-clock-v1";
const MESSAGE_TYPE = "playground.message";
const MESSAGE_TIMESTAMP_NS = 42n;
const MESSAGE_BYTES = new TextEncoder().encode("hello from the standard protocol playground");
const STREAM_RESOURCE_ID = "playground/scalar";
const STREAM_TIMESTAMP_NS = 99n;
const STREAM_VALUE = 12.5;
const REGISTRY_ID = "playground/base";
const REGISTRY_HASH = "0".repeat(32);
export const CHECK_NAMES = ["info", "catalog", "registry", "blob", "message", "stream"] as const;

export interface PeerCard {
  version: 1;
  runtime: string;
  domainId: string;
  peerId: string;
  protocols: string[];
  routes: { tcp: string; wss: string };
}

export interface ProbeResult {
  ok: boolean;
  checks: Record<(typeof CHECK_NAMES)[number], boolean>;
  errors: Partial<Record<(typeof CHECK_NAMES)[number], string>>;
}

export interface DiscoveredPeer {
  peerId: string;
  routes: string[];
  servedProtocols: string[];
  expiresAt: string;
  source: string;
}

type Endpoint = { close(): Promise<void>; free(): void };
type Client = { free(): void };
type RuntimeTarget = { peerId: string; route: string };

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

function bytesEqual(left: Uint8Array, right: Uint8Array): boolean {
  return left.length === right.length && left.every((value, index) => value === right[index]);
}

function messageChannel(peerId: string): AukiMessageChannelResource {
  return {
    variant: "message_channel",
    owner_peer_id: peerId,
    resource_id: MESSAGE_RESOURCE_ID,
    clock: { peer_id: peerId, id: MESSAGE_CLOCK_ID, hash: MESSAGE_CLOCK_HASH },
  };
}

function scalarBytes(): Uint8Array {
  const bytes = new Uint8Array(9);
  bytes[0] = 0x09;
  new DataView(bytes.buffer).setFloat64(1, STREAM_VALUE, true);
  return bytes;
}

function streamManifest(): AukiStreamManifest {
  return {
    sensorId: "",
    sensorHash: "",
    clockPeerId: "",
    clockId: "",
    clockHash: "",
    frameId: "",
    frameHash: "",
    resourceId: STREAM_RESOURCE_ID,
    payload: "scalar",
    fromFrameId: "",
    fromFrameHash: "",
    toFrameId: "",
    toFrameHash: "",
    writerMode: "",
    expectedRateHz: 0,
    mapPeerId: "",
    mapId: "",
    mapHash: "",
  };
}

async function* scalarSource(): AsyncIterable<AukiStreamSourceItem> {
  yield { timestampNs: STREAM_TIMESTAMP_NS, payload: scalarBytes() };
}

export class BrowserPlayground {
  private readonly endpoints: Endpoint[] = [];
  private readonly clients: Client[] = [];
  private receiver?: ReturnType<AukiMessageEndpoint["declare"]>;
  private receiverTask?: Promise<void>;
  private closed = false;
  private info!: AukiInfoClient;
  private catalog!: AukiCatalogClient;
  private registry!: AukiRegistryClient;
  private blob!: AukiBlobClient;
  private message!: AukiMessageClient;
  private stream!: AukiStreamClient;

  private constructor(
    private readonly peer: AukiPeer,
    private readonly nodeName: string,
    private readonly onEvent: (message: string) => void,
  ) {}

  static async start(
    session: {
      startPeerWithDiscovery(domainId: string, mode: AukiDiscoveryMode): Promise<AukiPeer>;
    },
    domainId: string,
    nodeName: string,
    discoveryMode: AukiDiscoveryMode,
    onEvent: (message: string) => void = () => undefined,
  ): Promise<BrowserPlayground> {
    const peer = await session.startPeerWithDiscovery(domainId, discoveryMode);
    const playground = new BrowserPlayground(peer, nodeName, onEvent);
    try {
      playground.mount();
      return playground;
    } catch (error) {
      await playground.close().catch(() => undefined);
      throw error;
    }
  }

  private mount(): void {
    const infoEndpoint = AukiInfoEndpoint.mount(this.peer, (requester) => {
      this.validateRequester(requester);
      return {
        app: APP,
        appVersion: APP_VERSION,
        name: this.nodeName,
        sessionId: "playground-session",
        sessionClockId: MESSAGE_CLOCK_ID,
        sessionClockHash: MESSAGE_CLOCK_HASH,
        sessionNowNs: 0n,
        peerId: this.peer.peerId,
        appInstance: "browser",
      };
    });
    this.endpoints.push(infoEndpoint);

    const messageEndpoint = AukiMessageEndpoint.mount(this.peer);
    this.endpoints.push(messageEndpoint);
    this.receiver = messageEndpoint.declare(messageChannel(this.peer.peerId), 16);
    this.receiverTask = this.drainMessages();

    const catalogEndpoint = AukiCatalogEndpoint.mount(
      this.peer,
      (requester, _request: AukiCatalogResourcesRequest) => {
        this.validateRequester(requester);
        return { resources: [messageChannel(this.peer.peerId) as unknown as AukiCatalogResource] };
      },
      (requester) => {
        this.validateRequester(requester);
        return { resources: [] };
      },
    );
    this.endpoints.push(catalogEndpoint);

    const registryEndpoint = AukiRegistryEndpoint.mount(
      this.peer,
      (requester, request: AukiRegistryProviderRequest) => {
        this.validateRequester(requester);
        return request.op === "list"
          ? { op: "list" as const, entries: [{ id: REGISTRY_ID, hash: REGISTRY_HASH }] }
          : { op: "get" as const, entry: null };
      },
    );
    this.endpoints.push(registryEndpoint);

    const blobEndpoint = AukiBlobEndpoint.mount(
      this.peer,
      async (requester, request: AukiBlobProviderRequest) => {
        this.validateRequester(requester);
        if (request.sha256 !== BLOB_SHA256) return null;
        const start = Number(request.offset);
        const end = Math.min(BLOB_BYTES.length, start + request.maxLen);
        return { totalSize: BigInt(BLOB_BYTES.length), bytes: BLOB_BYTES.slice(start, end) };
      },
    );
    this.endpoints.push(blobEndpoint);

    const streamEndpoint = AukiStreamEndpoint.mount(this.peer, (requester, request) => {
      this.validateRequester(requester);
      if (
        request.sourcePeerId !== this.peer.peerId ||
        request.resourceId !== STREAM_RESOURCE_ID ||
        request.from?.kind !== "latest"
      ) {
        return { kind: "decline" as const, reason: { kind: "sensor_not_found" as const } };
      }
      return {
        kind: "accept" as const,
        payloadKind: "scalar" as const,
        manifest: streamManifest(),
        source: scalarSource(),
      };
    });
    this.endpoints.push(streamEndpoint);

    this.info = new AukiInfoClient(this.peer);
    this.catalog = new AukiCatalogClient(this.peer);
    this.registry = new AukiRegistryClient(this.peer);
    this.blob = new AukiBlobClient(this.peer);
    this.message = new AukiMessageClient(this.peer);
    this.stream = new AukiStreamClient(this.peer);
    this.clients.push(this.info, this.catalog, this.registry, this.blob, this.message, this.stream);
  }

  private validateRequester(requester: AukiAuthenticatedPeer): void {
    assert(requester.peerId.length > 0, "authenticated requester Peer ID is missing");
    assert(
      requester.domainIds.includes(this.peer.domainId),
      "authenticated requester does not share the selected Domain",
    );
  }

  card(): PeerCard {
    return {
      version: 1,
      runtime: "browser",
      domainId: this.peer.domainId,
      peerId: this.peer.peerId,
      protocols: this.requiredProtocols(),
      routes: { tcp: this.peer.tcpRoute, wss: this.peer.wssRoute },
    };
  }

  async discover(protocolId?: string): Promise<DiscoveredPeer[]> {
    const candidates = protocolId
      ? await this.peer.discoverProtocol(protocolId)
      : await this.peer.discover();
    return candidates.map((candidate) => {
      try {
        return {
          peerId: candidate.peerId,
          routes: candidate.routes,
          servedProtocols: candidate.servedProtocols,
          expiresAt: candidate.expiresAt,
          source: candidate.source,
        };
      } finally {
        candidate.free();
      }
    });
  }

  canProbeDiscovered(candidate: DiscoveredPeer): boolean {
    return this.missingProtocols(candidate).length === 0
      && candidate.routes.some((route) => route.includes("/wss/"));
  }

  probeDiscovered(candidate: DiscoveredPeer): Promise<ProbeResult> {
    const missing = this.missingProtocols(candidate);
    assert(
      missing.length === 0,
      `discovered peer ${candidate.peerId} does not advertise: ${missing.join(", ")}`,
    );
    const route = candidate.routes.find(
      (value) => value.includes("/wss/") && value.includes("/p2p-circuit/"),
    ) ?? candidate.routes.find((value) => value.includes("/wss/"));
    assert(route, `discovered peer ${candidate.peerId} has no browser-compatible WSS route`);
    return this.probeTarget({ peerId: candidate.peerId, route });
  }

  async probeAll(target: PeerCard): Promise<ProbeResult> {
    assert(target.domainId === this.peer.domainId, "target peer belongs to another Domain");
    return this.probeTarget({ peerId: target.peerId, route: target.routes.wss });
  }

  private async probeTarget(target: RuntimeTarget): Promise<ProbeResult> {
    const checks = Object.fromEntries(CHECK_NAMES.map((name) => [name, false])) as ProbeResult["checks"];
    const errors: ProbeResult["errors"] = {};
    const probes: Array<[(typeof CHECK_NAMES)[number], () => Promise<void>]> = [
      ["info", () => this.probeInfo(target)],
      ["catalog", () => this.probeCatalog(target)],
      ["registry", () => this.probeRegistry(target)],
      ["blob", () => this.probeBlob(target)],
      ["message", () => this.probeMessage(target)],
      ["stream", () => this.probeStream(target)],
    ];
    for (const [name, probe] of probes) {
      try {
        await this.withTimeout(probe);
        checks[name] = true;
      } catch (error) {
        errors[name] = error instanceof Error ? `${error.name}: ${error.message}` : String(error);
      }
    }
    return { ok: Object.keys(errors).length === 0, checks, errors };
  }

  private async withTimeout(operation: () => Promise<void>): Promise<void> {
    let timer: ReturnType<typeof setTimeout> | undefined;
    try {
      await Promise.race([
        operation(),
        new Promise<never>((_, reject) => {
          timer = setTimeout(() => reject(new Error("protocol probe timed out after 60s")), 60_000);
        }),
      ]);
    } finally {
      if (timer !== undefined) clearTimeout(timer);
    }
  }

  private target(target: RuntimeTarget): AukiExactTarget {
    return { peerId: target.peerId, route: target.route };
  }

  private requiredProtocols(): string[] {
    return [
      this.info.protocol,
      this.catalog.resourceProtocol,
      this.catalog.mapsProtocol,
      this.registry.protocol,
      this.blob.protocol,
      this.message.protocol,
      this.stream.protocol,
    ];
  }

  private missingProtocols(candidate: DiscoveredPeer): string[] {
    return this.requiredProtocols().filter(
      (protocol) => !candidate.servedProtocols.includes(protocol),
    );
  }

  private async probeInfo(target: RuntimeTarget): Promise<void> {
    const info = await this.info.fetchExact(this.target(target));
    assert(info.peerId === target.peerId, "Info returned the wrong Peer ID");
    assert(info.app === APP && info.appVersion === APP_VERSION, "Info fixture mismatch");
  }

  private async probeCatalog(target: RuntimeTarget): Promise<void> {
    const resources = await this.catalog.fetchResourcesExact(this.target(target), []);
    assert(resources.resources.length === 1, "Catalog v3 fixture row is missing");
    const channel = resources.resources[0] as unknown as AukiMessageChannelResource;
    assert(channel.owner_peer_id === target.peerId, "Catalog message owner mismatch");
    assert(channel.resource_id === MESSAGE_RESOURCE_ID, "Catalog message resource mismatch");
    const maps = await this.catalog.fetchMapsExact(this.target(target));
    assert(maps.resources.length === 0, "Catalog v4 fixture unexpectedly advertised maps");
  }

  private async probeRegistry(target: RuntimeTarget): Promise<void> {
    const entries = await this.registry.listExact(this.target(target), "frame");
    assert(entries.length === 1, "Registry fixture row is missing");
    assert(
      entries[0]?.id === REGISTRY_ID && /^[0-9a-f]{32}$/.test(entries[0]?.hash ?? ""),
      "Registry fixture mismatch",
    );
  }

  private async probeBlob(target: RuntimeTarget): Promise<void> {
    const receipt = await this.blob.fetchExact(this.target(target), BLOB_SHA256);
    assert(receipt.peerId === target.peerId, "Blob receipt Peer ID mismatch");
    assert(receipt.sha256 === BLOB_SHA256, "Blob receipt hash mismatch");
    assert(bytesEqual(receipt.bytes, BLOB_BYTES), "Blob receipt bytes mismatch");
    assert(receipt.relayed, "Blob did not use the relay circuit");
  }

  private async probeMessage(target: RuntimeTarget): Promise<void> {
    const sender = await this.message.openExact(this.target(target), messageChannel(target.peerId));
    try {
      assert(sender.remotePeer.peerId === target.peerId, "Message Peer ID mismatch");
      assert(sender.relayed, "Message did not use the relay circuit");
      await sender.send(MESSAGE_TYPE, MESSAGE_TIMESTAMP_NS, MESSAGE_BYTES);
    } finally {
      try { await sender.close(); } finally { sender.free(); }
    }
  }

  private async probeStream(target: RuntimeTarget): Promise<void> {
    const request: AukiStreamRequest = {
      sourcePeerId: target.peerId,
      resourceId: STREAM_RESOURCE_ID,
      from: { kind: "latest" },
    };
    const subscription = await this.stream.subscribeExact(this.target(target), "scalar", request);
    try {
      assert(subscription.payloadKind === "scalar", "Stream payload kind mismatch");
      assert(subscription.manifest.resourceId === STREAM_RESOURCE_ID, "Stream resource mismatch");
      const entry = await subscription.next();
      assert(entry?.kind === "entry", "Stream fixture entry is missing");
      assert(entry.entry.sequence === 0n, "Stream sequence mismatch");
      assert(entry.entry.timestampNs === STREAM_TIMESTAMP_NS, "Stream timestamp mismatch");
      assert(bytesEqual(entry.entry.payload, scalarBytes()), "Stream scalar payload mismatch");
    } finally {
      try { await subscription.cancel(); } finally { subscription.free(); }
    }
  }

  private async drainMessages(): Promise<void> {
    const receiver = this.receiver;
    if (!receiver) return;
    while (true) {
      const event = await receiver.next();
      if (event === null) return;
      assert(event.type === MESSAGE_TYPE, "received Message type mismatch");
      assert(event.timestampNs === MESSAGE_TIMESTAMP_NS, "received Message timestamp mismatch");
      assert(bytesEqual(event.payload, MESSAGE_BYTES), "received Message payload mismatch");
      assert(event.sender.peerId.length > 0, "received Message sender is missing");
      this.onEvent(`message received from ${event.sender.peerId}: ${event.type}`);
    }
  }

  async close(): Promise<void> {
    if (this.closed) return;
    this.closed = true;
    const errors: string[] = [];
    if (this.receiver) {
      try { await this.receiver.close(); } catch (error) { errors.push(`Message receiver: ${error}`); }
      this.receiver.free();
      this.receiver = undefined;
    }
    if (this.receiverTask) {
      const [result] = await Promise.allSettled([this.receiverTask]);
      if (result?.status === "rejected") errors.push(`Message drain: ${result.reason}`);
    }
    for (const endpoint of this.endpoints.reverse()) {
      try { await endpoint.close(); } catch (error) { errors.push(`endpoint: ${error}`); }
      endpoint.free();
    }
    for (const client of this.clients) client.free();
    try { await this.peer.shutdown(); } catch (error) { errors.push(`Auki peer: ${error}`); }
    this.peer.free();
    if (errors.length) throw new Error(`ordered shutdown failed: ${errors.join("; ")}`);
  }
}
