import {
  type AukiBrowserBootstrapRecord,
  parseBootstrapRecord,
  preferredDialAddresses,
  relayServerAddresses,
} from "./bootstrap.js";
import { type SeedStore, indexedDbSeedStore, loadOrCreateSeed, peerIdFromSeed } from "./identity.js";
import { type ProtocolWasmInitInput, initializeProtocolWasm } from "./protocol.js";
import {
  type BrowserTransport,
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
};

export type SubscribeRequest = {
  peerId: string;
  domainId: string;
  offerId: string;
  params?: unknown;
};

export type SpatialMessage = Record<string, unknown>;

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
  return new DefaultAukiBrowserPeer(peerId, transport, config.bootstrap);
}

class DefaultAukiBrowserPeer implements AukiBrowserPeer {
  readonly supportedTransports = supportedBrowserTransports();
  private readonly peers = new Map<string, PeerSummary>();
  private started = false;

  constructor(
    readonly peerId: string,
    private readonly transport: BrowserTransport,
    bootstrap: unknown,
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
      this.rememberBootstrap(record);
      await this.transport.dial(preferredDialAddresses(record));
      this.peers.set(record.peerId, {
        peerId: record.peerId,
        connected: true,
        dialAddresses: preferredDialAddresses(record),
      });
    }
  }

  listPeers(): PeerSummary[] {
    return Array.from(this.peers.values());
  }

  async listOffers(_peerId?: string): Promise<OfferSummary[]> {
    return [];
  }

  async *subscribe(_request: SubscribeRequest): AsyncIterable<SpatialMessage> {
    throw new Error("Subscribe is not implemented in auki-p2p-browser yet");
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
}

function requiredSeed(seed: Uint8Array | undefined): Uint8Array {
  if (!seed) {
    throw new Error("A browser peer seed is required");
  }
  return seed;
}
