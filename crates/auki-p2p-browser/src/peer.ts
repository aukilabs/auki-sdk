import {
  type AukiBrowserBootstrapRecord,
  createLocalBootstrapRecord,
  parseBootstrapRecord,
  parseBootstrapRecords,
  preferredDialAddresses,
  relayServerAddresses,
} from "./bootstrap.js";
import { type SeedStore, indexedDbSeedStore, loadOrCreateSeed, peerIdFromSeed } from "./identity.js";
import {
  createDomainDeclaration,
  createGetRequest,
  createOfferCatalogRequest,
  createPeerBinding,
  createPeerHandshake,
  createSubscribeRequest,
  parseGetRequest,
  parseGetResponse,
  parseOfferCatalogRequest,
  parseOfferCatalogResponse,
  parsePeerHandshake,
  parseSubscribeRequest,
  parseSubscribeStartResult,
  verifyDomainDeclaration,
  validateGetResponseForRequest,
  validatePeerHandshakeAuthority,
  validateSubscribeDataMessage,
  validateSubscribeEndForOffer,
  validateSubscribeStartForRequest,
  ProtocolWasmError,
  type JsonObject,
  type ProtocolWasmInitInput,
  initializeProtocolWasm,
} from "./protocol.js";
import {
  createLocalOfferCatalogResponse,
  createGetFailure,
  createGetSuccess,
  createPublicationSpatialMessage,
  createPublishedOffer,
  createSubscribeAccept,
  createSubscribeEnd,
  createSubscribeEndForPath,
  createSubscribeReject,
  offerKey,
  offerSummary,
  optionalNumberField,
  requestAcceptsPayload,
  stringField,
  toAsyncIterable,
  type ByteSourceInput,
  type LoadedOffer,
  type LocalOfferPublication,
  type OfferSummary,
  type PublicationHandle,
  type PublishOfferOptions,
} from "./publication.js";
import { JsonFrameReader, writeJsonFrame } from "./stream.js";
import {
  type BrowserTransport,
  type BrowserConnectionPath,
  type BrowserProtocolStream,
  createBrowserLibp2pTransport,
  supportedBrowserTransports,
} from "./transport.js";

export type PeerSummary = {
  peerId: string;
  connected: boolean;
  dialAddresses: string[];
  connectionPaths: BrowserConnectionPath[];
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
  signal?: AbortSignal;
};

export type GetRequest = {
  peerId: string;
  domainId: string;
  offerId: string;
  params?: JsonObject;
  acceptedPayloadTypes?: string[];
  maxPayloadBytes?: number;
};

export type SpatialMessage = JsonObject;

export type AukiBrowserSubscription = {
  readonly messages: AsyncIterable<SpatialMessage>;
  stop(): Promise<void>;
};

export type AukiBrowserPeerTraceEvent = {
  at: string;
  operation: "get" | "subscribe";
  phase:
    | "dialing"
    | "opened"
    | "request_sent"
    | "response_received"
    | "start_received"
    | "accepted"
    | "completed"
    | "stream_closed"
    | "retrying"
    | "failed";
  peerId: string;
  protocol: string;
  attempt: number;
  domainId?: string;
  offerId?: string;
  error?: string;
  retryable?: boolean;
  nextAttempt?: number;
};

export type AukiBrowserPeerTraceSink = (event: AukiBrowserPeerTraceEvent) => void;

export type AukiBrowserPeerConfig = {
  seed?: Uint8Array;
  seedStore?: SeedStore;
  protocolWasm?: ProtocolWasmInitInput;
  transport?: BrowserTransport;
  bootstrap?: unknown;
  label?: string;
  trace?: AukiBrowserPeerTraceSink;
};

export type CreateLocalDomainOptions = {
  nonce?: Uint8Array;
  label?: string;
  metadata?: JsonObject;
};

export type AukiBrowserLocalDomain = {
  readonly domainId: string;
  readonly declaration: JsonObject;
  readonly metadata?: JsonObject;
};

export interface AukiBrowserPeer {
  readonly peerId: string;
  readonly supportedTransports: readonly string[];
  multiaddrs(): string[];
  localBootstrapRecord(): Promise<AukiBrowserBootstrapRecord>;
  dial(address: string): Promise<void>;
  connectBootstrap(records: unknown | unknown[]): Promise<void>;
  switchPeerAddress(peerId: string, address: string): Promise<void>;
  listPeers(): PeerSummary[];
  listOffers(peerId?: string): Promise<OfferSummary[]>;
  createLocalDomain(options?: CreateLocalDomainOptions): Promise<AukiBrowserLocalDomain>;
  get(request: GetRequest): Promise<SpatialMessage>;
  openSubscription(request: SubscribeRequest): Promise<AukiBrowserSubscription>;
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
        ? uniqueStrings(parseBootstrapRecords(config.bootstrap).flatMap(relayServerAddresses))
        : [],
    }));
  const peerId = config.transport ? config.transport.peerId : await peerIdFromSeed(requiredSeed(seed));
  if (transport.peerId !== peerId) {
    throw new Error(`Browser transport peer id ${transport.peerId} does not match expected ${peerId}`);
  }
  await initializeProtocolWasm(config.protocolWasm);
  return new DefaultAukiBrowserPeer(
    peerId,
    transport,
    config.bootstrap,
    seed,
    config.label,
    config.trace,
  );
}

const LIFECYCLE_PROTOCOL_ID = "/auki/cluster-lifecycle/0.0.1";
const OFFER_CATALOG_PROTOCOL_ID = "/auki/offer-catalog/0.0.1";
const GET_PROTOCOL_ID = "/auki/get/0.0.1";
const SUBSCRIBE_PROTOCOL_ID = "/auki/subscribe/0.0.1";
const SUBSCRIBE_END_TYPE = "auki.subscribe_end.v1";
const DEFAULT_FRAME_BODY_LIMIT = 1_048_576;
const DOMAIN_NONCE_BYTES = 16;
const GET_CLIENT_RETRY_ATTEMPTS = 2;
const SUBSCRIBE_CLIENT_RETRY_ATTEMPTS = 2;
const SUBSCRIBE_STOP_TIMEOUT_MS = 1_000;

class DefaultAukiBrowserPeer implements AukiBrowserPeer {
  readonly supportedTransports = supportedBrowserTransports();
  private readonly peers = new Map<string, PeerSummary>();
  private readonly lifecyclePeers = new Set<string>();
  private readonly remoteOffers = new Map<string, LoadedOffer[]>();
  private readonly localPublications = new Map<string, LocalOfferPublication>();
  private readonly localDomains = new Map<string, AukiBrowserLocalDomain>();
  private started = false;
  private inboundHandlersRegistered = false;

  constructor(
    readonly peerId: string,
    private readonly transport: BrowserTransport,
    bootstrap: unknown,
    private readonly walletSeed?: Uint8Array,
    private readonly label?: string,
    private readonly trace?: AukiBrowserPeerTraceSink,
  ) {
    if (bootstrap) {
      for (const record of parseBootstrapRecords(bootstrap)) {
        this.rememberBootstrap(record);
      }
    }
  }

  multiaddrs(): string[] {
    return this.transport.multiaddrs();
  }

  async localBootstrapRecord(): Promise<AukiBrowserBootstrapRecord> {
    await this.ensureStarted();
    return createLocalBootstrapRecord(this.peerId, this.transport.multiaddrs());
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
      this.transport.addRelayServerAddresses?.(relayServerAddresses(record));
      await this.transport.dial(dialAddresses);
      await this.exchangeLifecycle(record);
      this.peers.set(record.peerId, {
        peerId: record.peerId,
        connected: true,
        dialAddresses,
        connectionPaths: this.connectionPaths(record.peerId),
      });
    }
  }

  async switchPeerAddress(peerId: string, address: string): Promise<void> {
    await this.ensureStarted();
    const peer = this.requirePeer(peerId);
    if (!peer.dialAddresses.includes(address)) {
      throw new Error(`Address is not known for peer ${peerId}`);
    }

    const dialAddresses = uniqueStrings([
      address,
      ...peer.dialAddresses.filter((candidate) => candidate !== address),
    ]);
    const fallbackAddresses = uniqueStrings([
      ...this.connectionPaths(peerId)
        .map((path) => path.remoteAddress)
        .filter((candidate) => candidate !== address),
      ...peer.dialAddresses.filter((candidate) => candidate !== address),
    ]);

    try {
      await this.transport.dial([address], { force: true });
    } catch (firstError) {
      if (this.selectedAddressIsActive(peerId, address)) {
        await this.transport.closePeerConnections?.(peerId, [address]);
        this.rememberSwitchedPeer(peerId, dialAddresses);
        return;
      }
      await this.transport.closePeerConnections?.(peerId, []);
      try {
        await this.transport.dial([address], { force: true });
      } catch (secondError) {
        if (this.selectedAddressIsActive(peerId, address)) {
          await this.transport.closePeerConnections?.(peerId, [address]);
          this.rememberSwitchedPeer(peerId, dialAddresses);
          return;
        }
        await reconnectPeerBestEffort(this.transport, fallbackAddresses);
        throw new Error(
          `Switch to selected address failed: ${describeError(secondError)}; first attempt: ${describeError(firstError)}`,
        );
      }
    }
    await this.transport.closePeerConnections?.(peerId, [address]);
    this.rememberSwitchedPeer(peerId, dialAddresses);
  }

  private selectedAddressIsActive(peerId: string, address: string): boolean {
    return this.connectionPaths(peerId).some((path) => path.remoteAddress === address);
  }

  private rememberSwitchedPeer(peerId: string, dialAddresses: string[]): void {
    this.peers.set(peerId, {
      peerId,
      connected: true,
      dialAddresses,
      connectionPaths: this.connectionPaths(peerId),
    });
  }

  listPeers(): PeerSummary[] {
    return Array.from(this.peers.values()).map((peer) => ({
      ...peer,
      connectionPaths: this.connectionPaths(peer.peerId),
    }));
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

  async get(request: GetRequest): Promise<SpatialMessage> {
    await this.ensureStarted();
    const peer = this.requirePeer(request.peerId);
    const offer = await this.requireRemoteOffer(peer, request.domainId, request.offerId);
    if (!offer.accessModes.includes("get")) {
      throw new Error(`Offer ${request.domainId}/${request.offerId} does not advertise Get`);
    }
    const selectedPayloadType = offer.payloadType ?? request.acceptedPayloadTypes?.[0];
    if (!selectedPayloadType) {
      throw new Error(`Offer ${request.domainId}/${request.offerId} has no payload type`);
    }

    const getRequest = await createGetRequest(
      request.domainId,
      request.offerId,
      request.params,
      request.acceptedPayloadTypes ?? [selectedPayloadType],
      request.maxPayloadBytes,
    );

    return withRetry(
      GET_CLIENT_RETRY_ATTEMPTS,
      undefined,
      (attempt) =>
        this.getOnce(peer, getRequest, selectedPayloadType, {
          attempt,
          domainId: request.domainId,
          offerId: request.offerId,
        }),
      {
        onRetry: (error, attempt, nextAttempt) =>
          this.traceClientRetry("get", peer.peerId, GET_PROTOCOL_ID, request, {
            error,
            attempt,
            nextAttempt,
          }),
        onFailed: (error, attempt, retryable) =>
          this.traceClientFailure("get", peer.peerId, GET_PROTOCOL_ID, request, {
            error,
            attempt,
            retryable,
          }),
      },
    );
  }

  private async getOnce(
    peer: PeerSummary,
    getRequest: JsonObject,
    selectedPayloadType: string,
    trace: {
      attempt: number;
      domainId: string;
      offerId: string;
    },
  ): Promise<SpatialMessage> {
    const base = {
      operation: "get" as const,
      peerId: peer.peerId,
      protocol: GET_PROTOCOL_ID,
      attempt: trace.attempt,
      domainId: trace.domainId,
      offerId: trace.offerId,
    };
    this.emitTrace({ ...base, phase: "dialing" });
    let stream: BrowserProtocolStream | undefined;

    try {
      stream = await this.transport.dialProtocol(
        peer.peerId,
        peer.dialAddresses,
        GET_PROTOCOL_ID,
      );
      this.emitTrace({ ...base, phase: "opened" });
      const reader = new JsonFrameReader(stream);
      await writeJsonFrame(stream, getRequest, DEFAULT_FRAME_BODY_LIMIT);
      this.emitTrace({ ...base, phase: "request_sent" });
      const frame = await reader.read(DEFAULT_FRAME_BODY_LIMIT);
      this.emitTrace({ ...base, phase: "response_received" });
      const response = await parseGetResponse(frame.value);
      const message = await validateGetResponseForRequest(
        getRequest,
        response,
        selectedPayloadType,
      );
      this.emitTrace({ ...base, phase: "completed" });
      return message;
    } finally {
      if (stream) {
        void closeStreamQuietly(stream).finally(() => {
          this.emitTrace({ ...base, phase: "stream_closed" });
        });
      }
    }
  }

  async openSubscription(request: SubscribeRequest): Promise<AukiBrowserSubscription> {
    if (request.signal?.aborted) {
      throw abortReason(request.signal);
    }
    await this.ensureStarted();
    const peer = this.requirePeer(request.peerId);
    if (!this.remoteOffers.has(peer.peerId)) {
      await this.loadOffers(peer);
    }
    if (request.signal?.aborted) {
      throw abortReason(request.signal);
    }

    const subscribeRequest = await createSubscribeRequest(
      request.domainId,
      request.offerId,
      request.params,
      request.acceptedPayloadTypes ?? [],
      request.maxMessageBytes,
    );
    return withRetry(
      SUBSCRIBE_CLIENT_RETRY_ATTEMPTS,
      request.signal,
      (attempt) => this.openSubscriptionOnce(peer, request, subscribeRequest, attempt),
      {
        onRetry: (error, attempt, nextAttempt) =>
          this.traceClientRetry("subscribe", peer.peerId, SUBSCRIBE_PROTOCOL_ID, request, {
            error,
            attempt,
            nextAttempt,
          }),
        onFailed: (error, attempt, retryable) =>
          this.traceClientFailure("subscribe", peer.peerId, SUBSCRIBE_PROTOCOL_ID, request, {
            error,
            attempt,
            retryable,
          }),
      },
    );
  }

  private async openSubscriptionOnce(
    peer: PeerSummary,
    request: SubscribeRequest,
    subscribeRequest: JsonObject,
    attempt: number,
  ): Promise<AukiBrowserSubscription> {
    const base = {
      operation: "subscribe" as const,
      peerId: peer.peerId,
      protocol: SUBSCRIBE_PROTOCOL_ID,
      attempt,
      domainId: request.domainId,
      offerId: request.offerId,
    };
    this.emitTrace({ ...base, phase: "dialing" });
    const stream = await this.transport.dialProtocol(
      peer.peerId,
      peer.dialAddresses,
      SUBSCRIBE_PROTOCOL_ID,
    );
    this.emitTrace({ ...base, phase: "opened" });
    const cleanupAbort = bindStreamAbort(request.signal, stream);
    const reader = new JsonFrameReader(stream);

    try {
      if (request.signal?.aborted) {
        throw abortReason(request.signal);
      }
      await writeJsonFrame(stream, subscribeRequest, DEFAULT_FRAME_BODY_LIMIT, {
        signal: request.signal,
      });
      this.emitTrace({ ...base, phase: "request_sent" });
      const startFrame = await reader.read(DEFAULT_FRAME_BODY_LIMIT, {
        signal: request.signal,
      });
      this.emitTrace({ ...base, phase: "start_received" });
      const startResult = await parseSubscribeStartResult(startFrame.value);
      const startValidation = await validateSubscribeStartForRequest(
        subscribeRequest,
        startResult,
      );
      if (startValidation.accepted !== true) {
        const code = nestedString(startValidation, ["reject", "error", "code"]) ?? "unknown";
        throw new Error(`Subscribe rejected by ${peer.peerId}: ${code}`);
      }

      cleanupAbort();
      this.emitTrace({ ...base, phase: "accepted" });
      return new DefaultAukiBrowserSubscription(
        stream,
        reader,
        request,
        startResult,
        () => undefined,
      );
    } catch (error) {
      cleanupAbort();
      await closeStreamQuietly(stream);
      this.emitTrace({ ...base, phase: "stream_closed" });
      throw error;
    }
  }

  async *subscribe(request: SubscribeRequest): AsyncIterable<SpatialMessage> {
    let subscription: AukiBrowserSubscription;
    try {
      subscription = await this.openSubscription(request);
    } catch (error) {
      if (request.signal?.aborted) {
        return;
      }
      throw error;
    }
    try {
      for await (const message of subscription.messages) {
        yield message;
      }
    } finally {
      await subscription.stop();
    }
  }

  async createLocalDomain(
    options: CreateLocalDomainOptions = {},
  ): Promise<AukiBrowserLocalDomain> {
    await this.ensureStarted();
    const declaration = await createDomainDeclaration(
      requiredSeed(this.walletSeed),
      options.nonce ?? randomBytes(DOMAIN_NONCE_BYTES),
      options.label ?? this.label ?? "browser-domain",
    );
    const verified = await verifyDomainDeclaration(declaration);
    const domainId = stringField(verified, "domain_id");
    const domain: AukiBrowserLocalDomain = {
      domainId,
      declaration,
      ...(options.metadata ? { metadata: options.metadata } : {}),
    };
    this.localDomains.set(domainId, domain);
    this.lifecyclePeers.clear();
    return domain;
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
      domainId: offer.domainId,
      offerId: offer.offerId,
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
      connectionPaths: [],
    });
  }

  private connectionPaths(peerId: string): BrowserConnectionPath[] {
    return this.transport.connectionPaths?.(peerId) ?? [];
  }

  private async exchangeLifecycle(record: AukiBrowserBootstrapRecord): Promise<void> {
    if (!this.walletSeed || this.lifecyclePeers.has(record.peerId)) {
      return;
    }

    const now = new Date().toISOString();
    const handshake = await this.localPeerHandshake(now);
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

  private async requireRemoteOffer(
    peer: PeerSummary,
    domainId: string,
    offerId: string,
  ): Promise<LoadedOffer> {
    const offers = await this.loadOffers(peer);
    const offer = offers.find(
      (candidate) => candidate.domainId === domainId && candidate.offerId === offerId,
    );
    if (!offer) {
      throw new Error(`Unknown offer ${domainId}/${offerId} for peer ${peer.peerId}`);
    }
    return offer;
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
      LIFECYCLE_PROTOCOL_ID,
      (stream, remotePeerId) => this.serveLifecycle(stream, remotePeerId),
      { maxInboundStreams: 32 },
    );
    await this.transport.registerProtocolHandler(
      OFFER_CATALOG_PROTOCOL_ID,
      (stream) => this.serveOfferCatalog(stream),
      { maxInboundStreams: 32 },
    );
    await this.transport.registerProtocolHandler(
      GET_PROTOCOL_ID,
      (stream) => this.serveGet(stream),
      { maxInboundStreams: 64 },
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
    await this.transport.unregisterProtocolHandler(LIFECYCLE_PROTOCOL_ID);
    await this.transport.unregisterProtocolHandler(OFFER_CATALOG_PROTOCOL_ID);
    await this.transport.unregisterProtocolHandler(GET_PROTOCOL_ID);
    await this.transport.unregisterProtocolHandler(SUBSCRIBE_PROTOCOL_ID);
    this.inboundHandlersRegistered = false;
  }

  private async serveLifecycle(
    stream: BrowserProtocolStream,
    remotePeerId: string,
  ): Promise<void> {
    if (!this.walletSeed) {
      await closeStream(stream);
      return;
    }
    const now = new Date().toISOString();
    const reader = new JsonFrameReader(stream);
    try {
      const localHandshake = await this.localPeerHandshake(now);
      await writeJsonFrame(stream, localHandshake, DEFAULT_FRAME_BODY_LIMIT);
      const remoteFrame = await reader.read(DEFAULT_FRAME_BODY_LIMIT);
      const remoteHandshake = await parsePeerHandshake(remoteFrame.value);
      await validatePeerHandshakeAuthority(remoteHandshake, remotePeerId, true, now);
      this.lifecyclePeers.add(remotePeerId);
      this.peers.set(remotePeerId, {
        peerId: remotePeerId,
        connected: true,
        dialAddresses: [],
        connectionPaths: this.connectionPaths(remotePeerId),
      });
    } finally {
      await closeStream(stream);
    }
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

  private async serveGet(stream: BrowserProtocolStream): Promise<void> {
    const reader = new JsonFrameReader(stream);
    let request: JsonObject | undefined;
    try {
      const frame = await reader.read(DEFAULT_FRAME_BODY_LIMIT);
      request = await parseGetRequest(frame.value);
      const publication = this.localPublications.get(
        offerKey(stringField(request, "domain_id"), stringField(request, "offer_id")),
      );

      if (!publication || publication.stopped) {
        await writeJsonFrame(
          stream,
          await createGetFailure(request, "offer.unknown_offer"),
          DEFAULT_FRAME_BODY_LIMIT,
        );
        return;
      }
      if (!publication.offer.accessModes.includes("get")) {
        await writeJsonFrame(
          stream,
          await createGetFailure(request, "offer.unsupported_access_mode"),
          DEFAULT_FRAME_BODY_LIMIT,
        );
        return;
      }
      if (!requestAcceptsPayload(request, publication.offer.payloadType)) {
        await writeJsonFrame(
          stream,
          await createGetFailure(request, "offer.unsupported_payload_type"),
          DEFAULT_FRAME_BODY_LIMIT,
        );
        return;
      }

      const chunk = await firstChunk(publication.source);
      if (!chunk) {
        await writeJsonFrame(
          stream,
          await createGetFailure(request, "offer.temporarily_unavailable"),
          DEFAULT_FRAME_BODY_LIMIT,
        );
        return;
      }

      const selectedPayloadType = publication.offer.payloadType;
      if (!selectedPayloadType) {
        await writeJsonFrame(
          stream,
          await createGetFailure(request, "offer.unsupported_payload_type"),
          DEFAULT_FRAME_BODY_LIMIT,
        );
        return;
      }

      const message = await createPublicationSpatialMessage(publication, chunk);
      const response = await createGetSuccess(message);
      try {
        await validateGetResponseForRequest(request, response, selectedPayloadType);
      } catch (error) {
        await writeJsonFrame(
          stream,
          await createGetFailure(request, protocolFailureCode(error)),
          DEFAULT_FRAME_BODY_LIMIT,
        );
        return;
      }
      await writeJsonFrame(stream, response, DEFAULT_FRAME_BODY_LIMIT);
    } catch (error) {
      if (request) {
        await writeJsonFrame(
          stream,
          await createGetFailure(request, "transport.failed"),
          DEFAULT_FRAME_BODY_LIMIT,
        ).catch(() => undefined);
      }
      throw error;
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
      let consumerEnded = false;
      let consumerEndError: unknown;
      const consumerEndAbort = new AbortController();
      const consumerEnd = watchSubscribeConsumerEnd(
        reader,
        request,
        consumerEndAbort.signal,
        () => {
          consumerEnded = true;
        },
      )
        .then(() => {
          consumerEnded = true;
        })
        .catch((error: unknown) => {
          if (!consumerEndAbort.signal.aborted) {
            consumerEnded = true;
            consumerEndError = error;
          }
        });
      const consumerEndSignal = consumerEnd.then(() => ({ kind: "consumer-end" }) as const);
      const source = toAsyncIterable(publication.source)[Symbol.asyncIterator]();
      for (;;) {
        const next = await Promise.race([
          consumerEndSignal,
          source.next().then((result) => ({ kind: "chunk", result }) as const),
        ]);
        if (next.kind === "consumer-end") {
          break;
        }
        await Promise.resolve();
        if (consumerEndError) {
          throw consumerEndError;
        }
        if (consumerEnded) {
          break;
        }
        if (next.result.done) {
          break;
        }
        if (publication.stopped) {
          break;
        }
        const chunk = next.result.value;
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
      if (consumerEnded || publication.stopped) {
        await source.return?.();
      }
      consumerEndAbort.abort(new Error("producer stream ended"));
      await consumerEnd;
      if (consumerEndError) {
        throw consumerEndError;
      }

      if (!consumerEnded) {
        await writeJsonFrame(
          stream,
          await createSubscribeEnd(
            publication.offer,
            publication.stopped ? "offer_withdrawn" : "complete",
          ),
          DEFAULT_FRAME_BODY_LIMIT,
        );
      }
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

  private async localPeerHandshake(now: string): Promise<JsonObject> {
    const peerBinding = await createPeerBinding(
      requiredSeed(this.walletSeed),
      this.peerId,
      now,
      this.label ?? "browser-peer",
    );
    return createPeerHandshake(peerBinding, this.localDeclaredDomains());
  }

  private localDeclaredDomains(): JsonObject[] {
    return Array.from(this.localDomains.values()).map((domain) => {
      const declared: JsonObject = {
        domain_id: domain.domainId,
        domain_declaration: domain.declaration,
      };
      if (domain.metadata) {
        declared.metadata = { ...domain.metadata };
      }
      return declared;
    });
  }

  private localOfferSummaries(): OfferSummary[] {
    return Array.from(this.localPublications.values())
      .filter((publication) => !publication.stopped)
      .map(({ offer }) => {
        const { raw: _raw, ...summary } = offer;
        return summary;
      });
  }

  private traceClientRetry(
    operation: "get" | "subscribe",
    peerId: string,
    protocol: string,
    request: GetRequest | SubscribeRequest,
    retry: {
      error: unknown;
      attempt: number;
      nextAttempt: number;
    },
  ): void {
    this.emitTrace({
      operation,
      phase: "retrying",
      peerId,
      protocol,
      attempt: retry.attempt,
      domainId: request.domainId,
      offerId: request.offerId,
      error: errorMessage(retry.error),
      retryable: true,
      nextAttempt: retry.nextAttempt,
    });
  }

  private traceClientFailure(
    operation: "get" | "subscribe",
    peerId: string,
    protocol: string,
    request: GetRequest | SubscribeRequest,
    failure: {
      error: unknown;
      attempt: number;
      retryable: boolean;
    },
  ): void {
    this.emitTrace({
      operation,
      phase: "failed",
      peerId,
      protocol,
      attempt: failure.attempt,
      domainId: request.domainId,
      offerId: request.offerId,
      error: errorMessage(failure.error),
      retryable: failure.retryable,
    });
  }

  private emitTrace(event: Omit<AukiBrowserPeerTraceEvent, "at">): void {
    if (!this.trace) {
      return;
    }
    try {
      this.trace({
        at: new Date().toISOString(),
        ...event,
      });
    } catch {
      // Trace callbacks must not affect protocol behavior.
    }
  }
}

class DefaultAukiBrowserSubscription implements AukiBrowserSubscription {
  readonly messages: AsyncIterable<SpatialMessage>;
  private readonly readAbort = new AbortController();
  private stopPromise?: Promise<void>;
  private closed = false;

  constructor(
    private readonly stream: BrowserProtocolStream,
    private readonly reader: JsonFrameReader,
    private readonly request: SubscribeRequest,
    private readonly startResult: JsonObject,
    private cleanupRequestAbort: () => void,
  ) {
    this.messages = this.readMessages();
    if (request.signal) {
      const stopFromRequestAbort = () => {
        void this.stop();
      };
      request.signal.addEventListener("abort", stopFromRequestAbort, { once: true });
      const cleanup = this.cleanupRequestAbort;
      this.cleanupRequestAbort = () => {
        request.signal?.removeEventListener("abort", stopFromRequestAbort);
        cleanup();
      };
    }
  }

  async stop(): Promise<void> {
    if (this.stopPromise) {
      return this.stopPromise;
    }
    if (this.closed) {
      return;
    }

    this.closed = true;
    this.readAbort.abort(new Error("subscription stopped"));
    this.stopPromise = this.stopGracefully();
    return this.stopPromise;
  }

  private async *readMessages(): AsyncIterable<SpatialMessage> {
    try {
      for (;;) {
        if (this.closed) {
          return;
        }
        const frame = await this.reader.read(DEFAULT_FRAME_BODY_LIMIT, {
          signal: this.readAbort.signal,
        });
        if (frame.value.type === SUBSCRIBE_END_TYPE) {
          await validateSubscribeEndForOffer(
            frame.value,
            this.request.domainId,
            this.request.offerId,
          );
          this.closed = true;
          return;
        }
        yield await validateSubscribeDataMessage(
          this.startResult,
          frame.value,
          frame.bodyLength,
          this.request.maxMessageBytes,
        );
      }
    } catch (error) {
      if (this.closed || this.readAbort.signal.aborted) {
        return;
      }
      throw error;
    } finally {
      if (this.stopPromise) {
        await this.stopPromise;
      } else if (this.closed) {
        this.cleanupRequestAbort();
        await closeStreamQuietly(this.stream);
      } else {
        await this.stop();
      }
    }
  }

  private async stopGracefully(): Promise<void> {
    try {
      const end = await createSubscribeEndForPath(
        this.request.domainId,
        this.request.offerId,
        "cancelled",
      );
      const timeout = timeoutSignal(SUBSCRIBE_STOP_TIMEOUT_MS, "Subscribe stop timed out");
      try {
        await writeJsonFrame(this.stream, end, DEFAULT_FRAME_BODY_LIMIT, {
          signal: timeout.signal,
        });
      } catch {
        this.stream.abort?.(new Error("Subscribe cancel write failed"));
      } finally {
        timeout.clear();
      }
      await closeStreamWithTimeout(this.stream, SUBSCRIBE_STOP_TIMEOUT_MS);
    } finally {
      this.cleanupRequestAbort();
    }
  }
}

function requiredSeed(seed: Uint8Array | undefined): Uint8Array {
  if (!seed) {
    throw new Error("A browser peer seed is required");
  }
  return seed;
}

function uniqueStrings(values: string[]): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const value of values) {
    if (seen.has(value)) continue;
    seen.add(value);
    out.push(value);
  }
  return out;
}

function randomBytes(length: number): Uint8Array {
  const cryptoObject = globalThis.crypto;
  if (!cryptoObject?.getRandomValues) {
    throw new Error("crypto.getRandomValues is required to create a browser local domain");
  }
  const bytes = new Uint8Array(length);
  cryptoObject.getRandomValues(bytes);
  return bytes;
}

async function reconnectPeerBestEffort(
  transport: BrowserTransport,
  addresses: string[],
): Promise<void> {
  if (addresses.length === 0) {
    return;
  }
  await transport.dial(addresses, { force: true }).catch(() => undefined);
}

function describeError(error: unknown): string {
  if (error instanceof AggregateError) {
    const messages = error.errors.map(describeError).filter((message) => message.length > 0);
    return messages.length > 0 ? messages.join("; ") : error.message;
  }
  if (error instanceof Error) {
    const cause = "cause" in error ? (error as { cause?: unknown }).cause : undefined;
    return cause ? `${error.message}: ${describeError(cause)}` : error.message;
  }
  return String(error);
}

async function closeStream(stream: BrowserProtocolStream): Promise<void> {
  await stream.close();
}

async function closeStreamQuietly(stream: BrowserProtocolStream): Promise<void> {
  await stream.close().catch(() => undefined);
}

async function closeStreamWithTimeout(
  stream: BrowserProtocolStream,
  timeoutMs: number,
): Promise<void> {
  let timeout: ReturnType<typeof setTimeout> | undefined;
  try {
    await Promise.race([
      stream.close(),
      new Promise<never>((_, reject) => {
        timeout = setTimeout(
          () => reject(new Error(`stream close timed out after ${timeoutMs}ms`)),
          timeoutMs,
        );
      }),
    ]);
  } catch {
    stream.abort?.(new Error("stream close failed"));
  } finally {
    if (timeout) {
      clearTimeout(timeout);
    }
  }
}

function bindStreamAbort(
  signal: AbortSignal | undefined,
  stream: BrowserProtocolStream,
): () => void {
  if (!signal) {
    return () => undefined;
  }
  const abort = () => {
    const error = abortReason(signal);
    if (stream.closeRead) {
      void stream.closeRead().catch(() => undefined);
    }
    if (stream.abort) {
      stream.abort(error);
    } else {
      void closeStreamQuietly(stream);
    }
  };
  if (signal.aborted) {
    abort();
    return () => undefined;
  }
  signal.addEventListener("abort", abort, { once: true });
  return () => signal.removeEventListener("abort", abort);
}

function timeoutSignal(timeoutMs: number, message: string): {
  signal: AbortSignal;
  clear(): void;
} {
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(new Error(message)), timeoutMs);
  return {
    signal: controller.signal,
    clear: () => clearTimeout(timeout),
  };
}

function abortReason(signal: AbortSignal): Error {
  const reason = signal.reason;
  if (reason instanceof Error) {
    return reason;
  }
  if (typeof reason === "string") {
    return new Error(reason);
  }
  return new Error("subscription aborted");
}

async function withRetry<T>(
  retryAttempts: number,
  signal: AbortSignal | undefined,
  operation: (attempt: number) => Promise<T>,
  hooks: {
    onRetry?: (error: unknown, attempt: number, nextAttempt: number) => void;
    onFailed?: (error: unknown, attempt: number, retryable: boolean) => void;
  } = {},
): Promise<T> {
  let attempt = 1;
  for (;;) {
    try {
      return await operation(attempt);
    } catch (error) {
      if (signal?.aborted) {
        throw abortReason(signal);
      }
      const retryable = isRetryableTransportError(error);
      if (attempt > retryAttempts || !retryable) {
        hooks.onFailed?.(error, attempt, retryable);
        throw error;
      }
      hooks.onRetry?.(error, attempt, attempt + 1);
      attempt += 1;
      await yieldToEventLoop();
    }
  }
}

function isRetryableTransportError(error: unknown): boolean {
  if (!(error instanceof Error)) {
    return false;
  }
  const message = error.message.toLowerCase();
  return (
    message.includes("stream has been reset") ||
    message.includes("stream reset") ||
    message.includes("connection is closed") ||
    message.includes("stream closed") ||
    message.includes("protocol stream closed before a complete frame arrived") ||
    message.includes("unexpected eof")
  );
}

function yieldToEventLoop(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0));
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

async function firstChunk(source: ByteSourceInput): Promise<Uint8Array | undefined> {
  for await (const chunk of toAsyncIterable(source)) {
    return chunk;
  }
  return undefined;
}

async function watchSubscribeConsumerEnd(
  reader: JsonFrameReader,
  request: JsonObject,
  signal: AbortSignal,
  onFrame: () => void,
): Promise<void> {
  const frame = await reader.read(DEFAULT_FRAME_BODY_LIMIT, { signal });
  onFrame();
  await validateSubscribeEndForOffer(
    frame.value,
    stringField(request, "domain_id"),
    stringField(request, "offer_id"),
  );
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

function protocolFailureCode(error: unknown): string {
  return error instanceof ProtocolWasmError && error.failureCode
    ? error.failureCode
    : "transport.failed";
}
