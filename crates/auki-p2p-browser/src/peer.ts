import {
  type AukiBrowserBootstrapRecord,
  parseBootstrapRecord,
  preferredDialAddresses,
  relayServerAddresses,
} from "./bootstrap.js";
import { type SeedStore, indexedDbSeedStore, loadOrCreateSeed, peerIdFromSeed } from "./identity.js";
import {
  createOfferCatalogRequest,
  createPeerBinding,
  createPeerHandshake,
  createSubscribeRequest,
  parseOfferCatalogRequest,
  parseOfferCatalogResponse,
  parsePeerHandshake,
  parseSubscribeRequest,
  parseSubscribeStartResult,
  validatePeerHandshakeAuthority,
  validateSubscribeDataMessage,
  validateSubscribeEndForOffer,
  validateSubscribeStartForRequest,
  type JsonObject,
  type ProtocolWasmInitInput,
  initializeProtocolWasm,
} from "./protocol.js";
import {
  createLocalOfferCatalogResponse,
  createPublicationSpatialMessage,
  createPublishedOffer,
  createSubscribeAccept,
  createSubscribeEnd,
  createSubscribeReject,
  offerKey,
  offerSummary,
  optionalNumberField,
  requestAcceptsPayload,
  stringField,
  toAsyncIterable,
  type LoadedOffer,
  type LocalOfferPublication,
  type OfferSummary,
  type PublicationHandle,
  type PublishOfferOptions,
} from "./publication.js";
import { JsonFrameReader, writeJsonFrame } from "./stream.js";
import {
  type BrowserTransport,
  type BrowserProtocolStream,
  createBrowserLibp2pTransport,
  supportedBrowserTransports,
} from "./transport.js";

export type PeerSummary = {
  peerId: string;
  connected: boolean;
  dialAddresses: string[];
};

export type {
  ByteSource,
  ByteSourceFactory,
  ByteSourceInput,
  OfferSummary,
  PublicationHandle,
  PublishOfferOptions,
} from "./publication.js";

export type SubscribeRequest = {
  peerId: string;
  domainId: string;
  offerId: string;
  params?: JsonObject;
  acceptedPayloadTypes?: string[];
  maxMessageBytes?: number;
};

export type SpatialMessage = JsonObject;

export type AukiBrowserPeerConfig = {
  seed?: Uint8Array;
  seedStore?: SeedStore;
  protocolWasm?: ProtocolWasmInitInput;
  transport?: BrowserTransport;
  bootstrap?: unknown;
  label?: string;
};

export interface AukiBrowserPeer {
  readonly peerId: string;
  readonly supportedTransports: readonly string[];
  multiaddrs(): string[];
  dial(address: string): Promise<void>;
  connectBootstrap(records: unknown | unknown[]): Promise<void>;
  listPeers(): PeerSummary[];
  listOffers(peerId?: string): Promise<OfferSummary[]>;
  subscribe(request: SubscribeRequest): AsyncIterable<SpatialMessage>;
  publishOffer(options: PublishOfferOptions): Promise<PublicationHandle>;
  stop(): Promise<void>;
}

export async function createAukiBrowserPeer(
  config: AukiBrowserPeerConfig = {},
): Promise<AukiBrowserPeer> {
  const seedStore =
    config.seedStore ?? (!config.seed && !config.transport ? indexedDbSeedStore() : undefined);
  const seed = config.seed ?? (seedStore ? await loadOrCreateSeed(seedStore) : undefined);
  if (!seed && !config.transport) {
    throw new Error("A browser peer needs a seed, seedStore, or injected transport");
  }
  const transport =
    config.transport ??
    (await createBrowserLibp2pTransport({
      seed: requiredSeed(seed),
      relayServerAddresses: config.bootstrap
        ? relayServerAddresses(parseBootstrapRecord(config.bootstrap))
        : [],
    }));
  const peerId = config.transport ? config.transport.peerId : await peerIdFromSeed(requiredSeed(seed));
  if (transport.peerId !== peerId) {
    throw new Error(`Browser transport peer id ${transport.peerId} does not match expected ${peerId}`);
  }
  await initializeProtocolWasm(config.protocolWasm);
  return new DefaultAukiBrowserPeer(peerId, transport, config.bootstrap, seed, config.label);
}

const LIFECYCLE_PROTOCOL_ID = "/auki/cluster-lifecycle/0.0.1";
const OFFER_CATALOG_PROTOCOL_ID = "/auki/offer-catalog/0.0.1";
const SUBSCRIBE_PROTOCOL_ID = "/auki/subscribe/0.0.1";
const SUBSCRIBE_END_TYPE = "auki.subscribe_end.v1";
const DEFAULT_FRAME_BODY_LIMIT = 1_048_576;

class DefaultAukiBrowserPeer implements AukiBrowserPeer {
  readonly supportedTransports = supportedBrowserTransports();
  private readonly peers = new Map<string, PeerSummary>();
  private readonly lifecyclePeers = new Set<string>();
  private readonly remoteOffers = new Map<string, LoadedOffer[]>();
  private readonly localPublications = new Map<string, LocalOfferPublication>();
  private started = false;
  private inboundHandlersRegistered = false;

  constructor(
    readonly peerId: string,
    private readonly transport: BrowserTransport,
    bootstrap: unknown,
    private readonly walletSeed?: Uint8Array,
    private readonly label?: string,
  ) {
    if (bootstrap) {
      this.rememberBootstrap(parseBootstrapRecord(bootstrap));
    }
  }

  multiaddrs(): string[] {
    return this.transport.multiaddrs();
  }

  async dial(address: string): Promise<void> {
    await this.ensureStarted();
    await this.transport.dial([address]);
  }

  async connectBootstrap(records: unknown | unknown[]): Promise<void> {
    await this.ensureStarted();
    for (const value of Array.isArray(records) ? records : [records]) {
      const record = parseBootstrapRecord(value);
      const dialAddresses = preferredDialAddresses(record);
      this.rememberBootstrap(record);
      await this.transport.dial(dialAddresses);
      await this.exchangeLifecycle(record);
      this.peers.set(record.peerId, {
        peerId: record.peerId,
        connected: true,
        dialAddresses,
      });
    }
  }

  listPeers(): PeerSummary[] {
    return Array.from(this.peers.values());
  }

  async listOffers(peerId?: string): Promise<OfferSummary[]> {
    await this.ensureStarted();
    if (peerId === this.peerId) {
      return this.localOfferSummaries();
    }
    const peers = peerId ? [this.requirePeer(peerId)] : Array.from(this.peers.values());
    const offers = await Promise.all(peers.map((peer) => this.loadOffers(peer)));
    return [
      ...(peerId ? [] : this.localOfferSummaries()),
      ...offers.flat().map(({ raw: _raw, ...summary }) => summary),
    ];
  }

  async *subscribe(request: SubscribeRequest): AsyncIterable<SpatialMessage> {
    await this.ensureStarted();
    const peer = this.requirePeer(request.peerId);
    if (!this.remoteOffers.has(peer.peerId)) {
      await this.loadOffers(peer);
    }

    const subscribeRequest = await createSubscribeRequest(
      request.domainId,
      request.offerId,
      request.params,
      request.acceptedPayloadTypes ?? [],
      request.maxMessageBytes,
    );
    const stream = await this.transport.dialProtocol(
      peer.peerId,
      peer.dialAddresses,
      SUBSCRIBE_PROTOCOL_ID,
    );
    const reader = new JsonFrameReader(stream);

    try {
      await writeJsonFrame(stream, subscribeRequest, DEFAULT_FRAME_BODY_LIMIT);
      const startFrame = await reader.read(DEFAULT_FRAME_BODY_LIMIT);
      const startResult = await parseSubscribeStartResult(startFrame.value);
      const startValidation = await validateSubscribeStartForRequest(
        subscribeRequest,
        startResult,
      );
      if (startValidation.accepted !== true) {
        const code = nestedString(startValidation, ["reject", "error", "code"]) ?? "unknown";
        throw new Error(`Subscribe rejected by ${peer.peerId}: ${code}`);
      }

      for (;;) {
        const frame = await reader.read(DEFAULT_FRAME_BODY_LIMIT);
        if (frame.value.type === SUBSCRIBE_END_TYPE) {
          await validateSubscribeEndForOffer(frame.value, request.domainId, request.offerId);
          return;
        }
        yield await validateSubscribeDataMessage(
          startResult,
          frame.value,
          frame.bodyLength,
          request.maxMessageBytes,
        );
      }
    } finally {
      await closeStream(stream);
    }
  }

  async publishOffer(options: PublishOfferOptions): Promise<PublicationHandle> {
    await this.ensureStarted();
    const offer = await createPublishedOffer(this.peerId, options);
    const key = offerKey(offer.domainId, offer.offerId);
    if (this.localPublications.has(key)) {
      throw new Error(`Offer already published for ${offer.domainId}/${offer.offerId}`);
    }

    const publication: LocalOfferPublication = {
      source: options.source,
      offer,
      stopped: false,
      nextSequence: 0n,
    };
    this.localPublications.set(key, publication);

    return {
      stop: async () => {
        publication.stopped = true;
        this.localPublications.delete(key);
      },
    };
  }

  async stop(): Promise<void> {
    for (const publication of this.localPublications.values()) {
      publication.stopped = true;
    }
    this.localPublications.clear();
    await this.uninstallInboundHandlers();
    await this.transport.stop();
    this.started = false;
  }

  private async ensureStarted(): Promise<void> {
    if (!this.started) {
      await this.transport.start();
      this.started = true;
    }
    await this.installInboundHandlers();
  }

  private rememberBootstrap(record: AukiBrowserBootstrapRecord): void {
    this.peers.set(record.peerId, {
      peerId: record.peerId,
      connected: false,
      dialAddresses: preferredDialAddresses(record),
    });
  }

  private async exchangeLifecycle(record: AukiBrowserBootstrapRecord): Promise<void> {
    if (!this.walletSeed || this.lifecyclePeers.has(record.peerId)) {
      return;
    }

    const now = new Date().toISOString();
    const peerBinding = await createPeerBinding(
      this.walletSeed,
      this.peerId,
      now,
      this.label ?? "browser-peer",
    );
    const handshake = await createPeerHandshake(peerBinding);
    const stream = await this.transport.dialProtocol(
      record.peerId,
      preferredDialAddresses(record),
      LIFECYCLE_PROTOCOL_ID,
    );
    const reader = new JsonFrameReader(stream);

    try {
      await writeJsonFrame(stream, handshake, DEFAULT_FRAME_BODY_LIMIT);
      const remoteFrame = await reader.read(DEFAULT_FRAME_BODY_LIMIT);
      const remoteHandshake = await parsePeerHandshake(remoteFrame.value);
      await validatePeerHandshakeAuthority(remoteHandshake, record.peerId, true, now);
      this.lifecyclePeers.add(record.peerId);
    } finally {
      await closeStream(stream);
    }
  }

  private async loadOffers(peer: PeerSummary): Promise<LoadedOffer[]> {
    const cached = this.remoteOffers.get(peer.peerId);
    if (cached) {
      return cached;
    }

    const stream = await this.transport.dialProtocol(
      peer.peerId,
      peer.dialAddresses,
      OFFER_CATALOG_PROTOCOL_ID,
    );
    const reader = new JsonFrameReader(stream);

    try {
      await writeJsonFrame(stream, await createOfferCatalogRequest(), DEFAULT_FRAME_BODY_LIMIT);
      const frame = await reader.read(DEFAULT_FRAME_BODY_LIMIT);
      const catalog = await parseOfferCatalogResponse(frame.value);
      const rawOffers = Array.isArray(catalog.offers) ? catalog.offers : [];
      const offers = rawOffers.map((offer) => offerSummary(peer.peerId, offer));
      this.remoteOffers.set(peer.peerId, offers);
      return offers;
    } finally {
      await closeStream(stream);
    }
  }

  private requirePeer(peerId: string): PeerSummary {
    const peer = this.peers.get(peerId);
    if (!peer) {
      throw new Error(`Unknown peer ${peerId}; connect bootstrap records before using it`);
    }
    return peer;
  }

  private async installInboundHandlers(): Promise<void> {
    if (this.inboundHandlersRegistered) {
      return;
    }
    await this.transport.registerProtocolHandler(
      OFFER_CATALOG_PROTOCOL_ID,
      (stream) => this.serveOfferCatalog(stream),
      { maxInboundStreams: 32 },
    );
    await this.transport.registerProtocolHandler(
      SUBSCRIBE_PROTOCOL_ID,
      (stream) => this.serveSubscribe(stream),
      { maxInboundStreams: 64 },
    );
    this.inboundHandlersRegistered = true;
  }

  private async uninstallInboundHandlers(): Promise<void> {
    if (!this.inboundHandlersRegistered) {
      return;
    }
    await this.transport.unregisterProtocolHandler(OFFER_CATALOG_PROTOCOL_ID);
    await this.transport.unregisterProtocolHandler(SUBSCRIBE_PROTOCOL_ID);
    this.inboundHandlersRegistered = false;
  }

  private async serveOfferCatalog(stream: BrowserProtocolStream): Promise<void> {
    const reader = new JsonFrameReader(stream);
    try {
      const frame = await reader.read(DEFAULT_FRAME_BODY_LIMIT);
      const request = await parseOfferCatalogRequest(frame.value);
      await writeJsonFrame(stream, await this.localOfferCatalogResponse(request), DEFAULT_FRAME_BODY_LIMIT);
    } finally {
      await closeStream(stream);
    }
  }

  private async serveSubscribe(stream: BrowserProtocolStream): Promise<void> {
    const reader = new JsonFrameReader(stream);
    let request: JsonObject | undefined;
    let startSent = false;
    try {
      const frame = await reader.read(DEFAULT_FRAME_BODY_LIMIT);
      request = await parseSubscribeRequest(frame.value);
      const publication = this.localPublications.get(
        offerKey(stringField(request, "domain_id"), stringField(request, "offer_id")),
      );

      if (!publication || publication.stopped) {
        await writeJsonFrame(
          stream,
          await createSubscribeReject(request, "offer.unknown_offer"),
          DEFAULT_FRAME_BODY_LIMIT,
        );
        return;
      }
      if (!requestAcceptsPayload(request, publication.offer.payloadType)) {
        await writeJsonFrame(
          stream,
          await createSubscribeReject(request, "offer.unsupported_payload_type"),
          DEFAULT_FRAME_BODY_LIMIT,
        );
        return;
      }

      const accept = await createSubscribeAccept(publication.offer);
      const startValidation = await validateSubscribeStartForRequest(request, accept);
      if (startValidation.accepted !== true) {
        await writeJsonFrame(
          stream,
          await createSubscribeReject(request, "subscribe.invalid_request"),
          DEFAULT_FRAME_BODY_LIMIT,
        );
        return;
      }

      await writeJsonFrame(stream, accept, DEFAULT_FRAME_BODY_LIMIT);
      startSent = true;
      for await (const chunk of toAsyncIterable(publication.source)) {
        if (publication.stopped) {
          break;
        }
        const message = await createPublicationSpatialMessage(publication, chunk);
        const maxMessageBytes = optionalNumberField(request, "max_message_bytes");
        const validMessage = await validateSubscribeDataMessage(
          accept,
          message,
          undefined,
          maxMessageBytes,
        );
        await writeJsonFrame(stream, validMessage, DEFAULT_FRAME_BODY_LIMIT);
      }

      await writeJsonFrame(
        stream,
        await createSubscribeEnd(
          publication.offer,
          publication.stopped ? "offer_withdrawn" : "complete",
        ),
        DEFAULT_FRAME_BODY_LIMIT,
      );
    } catch (error) {
      if (request && !startSent) {
        await writeJsonFrame(
          stream,
          await createSubscribeReject(request, "transport.failed"),
          DEFAULT_FRAME_BODY_LIMIT,
        ).catch(() => undefined);
      }
      throw error;
    } finally {
      await closeStream(stream);
    }
  }

  private async localOfferCatalogResponse(request: JsonObject): Promise<JsonObject> {
    return createLocalOfferCatalogResponse(this.localPublications.values(), request);
  }

  private localOfferSummaries(): OfferSummary[] {
    return Array.from(this.localPublications.values())
      .filter((publication) => !publication.stopped)
      .map(({ offer }) => {
        const { raw: _raw, ...summary } = offer;
        return summary;
      });
  }
}

function requiredSeed(seed: Uint8Array | undefined): Uint8Array {
  if (!seed) {
    throw new Error("A browser peer seed is required");
  }
  return seed;
}

async function closeStream(stream: BrowserProtocolStream): Promise<void> {
  await stream.close();
}

function nestedString(value: unknown, path: string[]): string | undefined {
  let current = value;
  for (const segment of path) {
    if (!isJsonObject(current)) {
      return undefined;
    }
    current = current[segment];
  }
  return typeof current === "string" ? current : undefined;
}

function isJsonObject(value: unknown): value is JsonObject {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}
