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
  parseOfferCatalogResponse,
  parsePeerHandshake,
  parseSubscribeStartResult,
  validatePeerHandshakeAuthority,
  validateSubscribeDataMessage,
  validateSubscribeEndForOffer,
  validateSubscribeStartForRequest,
  type JsonObject,
  type ProtocolWasmInitInput,
  initializeProtocolWasm,
} from "./protocol.js";
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

export type OfferSummary = {
  peerId: string;
  domainId: string;
  offerId: string;
  kind?: string;
  payloadType?: string;
  accessModes: string[];
};

export type SubscribeRequest = {
  peerId: string;
  domainId: string;
  offerId: string;
  params?: JsonObject;
  acceptedPayloadTypes?: string[];
  maxMessageBytes?: number;
};

export type SpatialMessage = JsonObject;

export type PreviewSource = AsyncIterable<Uint8Array> | Iterable<Uint8Array>;

export type PreviewOfferOptions = {
  domainId: string;
  offerId: string;
  payloadType?: string;
};

export type PublicationHandle = {
  stop(): Promise<void>;
};

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
  publishPreview(source: PreviewSource, options: PreviewOfferOptions): Promise<PublicationHandle>;
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

type LoadedOffer = OfferSummary & {
  raw: JsonObject;
};

class DefaultAukiBrowserPeer implements AukiBrowserPeer {
  readonly supportedTransports = supportedBrowserTransports();
  private readonly peers = new Map<string, PeerSummary>();
  private readonly lifecyclePeers = new Set<string>();
  private readonly remoteOffers = new Map<string, LoadedOffer[]>();
  private started = false;

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
    const peers = peerId ? [this.requirePeer(peerId)] : Array.from(this.peers.values());
    const offers = await Promise.all(peers.map((peer) => this.loadOffers(peer)));
    return offers.flat().map(({ raw: _raw, ...summary }) => summary);
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
    const messageLimit = request.maxMessageBytes ?? DEFAULT_FRAME_BODY_LIMIT;

    try {
      await writeJsonFrame(stream, subscribeRequest, DEFAULT_FRAME_BODY_LIMIT);
      const startFrame = await reader.read(messageLimit);
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
        const frame = await reader.read(messageLimit);
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

  async publishPreview(
    _source: PreviewSource,
    _options: PreviewOfferOptions,
  ): Promise<PublicationHandle> {
    throw new Error("Preview publishing is not implemented in auki-p2p-browser yet");
  }

  async stop(): Promise<void> {
    await this.transport.stop();
    this.started = false;
  }

  private async ensureStarted(): Promise<void> {
    if (this.started) return;
    await this.transport.start();
    this.started = true;
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

function offerSummary(peerId: string, value: unknown): LoadedOffer {
  if (!isJsonObject(value)) {
    throw new Error("Offer catalog response contains a non-object offer");
  }
  const payload = isJsonObject(value.payload) ? value.payload : undefined;
  return {
    peerId,
    domainId: stringField(value, "domain_id"),
    offerId: stringField(value, "offer_id"),
    kind: optionalStringField(value, "kind"),
    payloadType: payload ? optionalStringField(payload, "type") : undefined,
    accessModes: stringArrayField(value, "access_modes"),
    raw: value,
  };
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

function stringField(value: JsonObject, field: string): string {
  const fieldValue = value[field];
  if (typeof fieldValue !== "string") {
    throw new Error(`Offer catalog response offer missing string field ${field}`);
  }
  return fieldValue;
}

function optionalStringField(value: JsonObject, field: string): string | undefined {
  const fieldValue = value[field];
  if (fieldValue === undefined) {
    return undefined;
  }
  if (typeof fieldValue !== "string") {
    throw new Error(`Offer catalog response offer field ${field} must be a string`);
  }
  return fieldValue;
}

function stringArrayField(value: JsonObject, field: string): string[] {
  const fieldValue = value[field];
  if (!Array.isArray(fieldValue) || fieldValue.some((item) => typeof item !== "string")) {
    throw new Error(`Offer catalog response offer field ${field} must be a string array`);
  }
  return fieldValue.slice();
}

function isJsonObject(value: unknown): value is JsonObject {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}
