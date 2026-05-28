import {
  createAukiBrowserPeer,
  type AukiBrowserPeer,
  type AukiBrowserPeerConfig,
  type AukiBrowserSubscription,
  type OfferSummary,
  type PeerSummary,
  type SpatialMessage,
} from "./peer.js";
import {
  parseBootstrapRecords,
  type AukiBrowserBootstrapRecord,
} from "./bootstrap.js";
import type { JsonObject } from "./protocol.js";
import {
  PREVIEW_PAYLOAD_TYPE,
  findPreviewOffer,
  isPreviewOffer,
  previewFrameFromMessage,
  type PreviewFrame,
} from "./preview.js";

export type PreviewReadOptions = {
  params?: JsonObject;
  acceptedPayloadTypes?: string[];
  maxPayloadBytes?: number;
};

export type PreviewSubscribeOptions = {
  params?: JsonObject;
  acceptedPayloadTypes?: string[];
  maxMessageBytes?: number;
  signal?: AbortSignal;
};

export type AukiPreviewSubscription = {
  readonly frames: AsyncIterable<PreviewFrame>;
  stop(): Promise<void>;
};

export type AukiPreviewBrowserSessionOptions = Omit<AukiBrowserPeerConfig, "bootstrap"> & {
  bootstrap: unknown;
  selectOffer?: (offer: OfferSummary) => boolean;
  acceptedPayloadTypes?: string[];
  maxPayloadBytes?: number;
  maxMessageBytes?: number;
};

export class AukiPreviewBrowserSession {
  readonly bootstrap: AukiBrowserBootstrapRecord;
  readonly bootstraps: AukiBrowserBootstrapRecord[];

  constructor(
    readonly peer: AukiBrowserPeer,
    bootstrap: AukiBrowserBootstrapRecord | AukiBrowserBootstrapRecord[],
    public peers: PeerSummary[],
    public offers: OfferSummary[],
    public previewOffer: OfferSummary | undefined,
    private readonly defaults: {
      acceptedPayloadTypes?: string[];
      maxPayloadBytes?: number;
      maxMessageBytes?: number;
      selectOffer?: (offer: OfferSummary) => boolean;
    } = {},
  ) {
    this.bootstraps = Array.isArray(bootstrap) ? bootstrap : [bootstrap];
    const [firstBootstrap] = this.bootstraps;
    if (!firstBootstrap) {
      throw new Error("A preview browser session needs at least one bootstrap record");
    }
    this.bootstrap = firstBootstrap;
  }

  async refreshOffers(): Promise<OfferSummary[]> {
    this.peers = this.peer.listPeers();
    this.offers = await this.peer.listOffers();
    this.previewOffer = findPreviewOffer(this.offers, this.defaults.selectOffer);
    return this.offers;
  }

  getSnapshot(
    offer = this.previewOffer,
    options: PreviewReadOptions = {},
  ): Promise<PreviewFrame> {
    return getPreviewSnapshot(this.peer, requirePreviewOffer(offer), {
      ...options,
      acceptedPayloadTypes:
        options.acceptedPayloadTypes ?? this.defaults.acceptedPayloadTypes,
      maxPayloadBytes: options.maxPayloadBytes ?? this.defaults.maxPayloadBytes,
    });
  }

  openSubscription(
    offer = this.previewOffer,
    options: PreviewSubscribeOptions = {},
  ): Promise<AukiPreviewSubscription> {
    return openPreviewSubscription(this.peer, requirePreviewOffer(offer), {
      ...options,
      acceptedPayloadTypes:
        options.acceptedPayloadTypes ?? this.defaults.acceptedPayloadTypes,
      maxMessageBytes: options.maxMessageBytes ?? this.defaults.maxMessageBytes,
    });
  }

  async stop(): Promise<void> {
    await this.peer.stop();
  }
}

export async function createAukiPreviewBrowserSession(
  options: AukiPreviewBrowserSessionOptions,
): Promise<AukiPreviewBrowserSession> {
  const {
    bootstrap: bootstrapInput,
    selectOffer,
    acceptedPayloadTypes,
    maxPayloadBytes,
    maxMessageBytes,
    ...peerConfig
  } = options;
  const bootstraps = parseBootstrapRecords(bootstrapInput);
  if (bootstraps.length === 0) {
    throw new Error("A preview browser session needs at least one bootstrap record");
  }
  const peer = await createAukiBrowserPeer(peerConfig);
  try {
    await peer.connectBootstrap(bootstraps);
    const offers = await peer.listOffers();
    return new AukiPreviewBrowserSession(
      peer,
      bootstraps,
      peer.listPeers(),
      offers,
      findPreviewOffer(offers, selectOffer),
      {
        acceptedPayloadTypes,
        maxPayloadBytes,
        maxMessageBytes,
        selectOffer,
      },
    );
  } catch (error) {
    await peer.stop().catch(() => undefined);
    throw error;
  }
}

export async function getPreviewSnapshot(
  peer: AukiBrowserPeer,
  offer: OfferSummary,
  options: PreviewReadOptions = {},
): Promise<PreviewFrame> {
  assertPreviewGetOffer(offer);
  const message = await peer.get({
    peerId: offer.peerId,
    domainId: offer.domainId,
    offerId: offer.offerId,
    params: options.params,
    acceptedPayloadTypes: options.acceptedPayloadTypes ?? [PREVIEW_PAYLOAD_TYPE],
    maxPayloadBytes: options.maxPayloadBytes ?? 1_048_576,
  });
  return previewFrameFromMessage(message);
}

export async function openPreviewSubscription(
  peer: AukiBrowserPeer,
  offer: OfferSummary,
  options: PreviewSubscribeOptions = {},
): Promise<AukiPreviewSubscription> {
  assertPreviewSubscribeOffer(offer);
  const subscription = await peer.openSubscription({
    peerId: offer.peerId,
    domainId: offer.domainId,
    offerId: offer.offerId,
    params: options.params,
    acceptedPayloadTypes: options.acceptedPayloadTypes ?? [PREVIEW_PAYLOAD_TYPE],
    maxMessageBytes: options.maxMessageBytes ?? 1_048_576,
    signal: options.signal,
  });
  return wrapPreviewSubscription(subscription);
}

export async function* subscribePreview(
  peer: AukiBrowserPeer,
  offer: OfferSummary,
  options: PreviewSubscribeOptions = {},
): AsyncIterable<PreviewFrame> {
  const subscription = await openPreviewSubscription(peer, offer, options);
  try {
    for await (const frame of subscription.frames) {
      yield frame;
    }
  } finally {
    await subscription.stop();
  }
}

function wrapPreviewSubscription(
  subscription: AukiBrowserSubscription,
): AukiPreviewSubscription {
  return {
    frames: previewFrames(subscription.messages),
    stop: () => subscription.stop(),
  };
}

async function* previewFrames(
  messages: AsyncIterable<SpatialMessage>,
): AsyncIterable<PreviewFrame> {
  for await (const message of messages) {
    yield previewFrameFromMessage(message);
  }
}

function assertPreviewGetOffer(offer: OfferSummary): void {
  if (!offer.accessModes.includes("get")) {
    throw new Error(`Offer ${offer.domainId}/${offer.offerId} does not advertise Get`);
  }
  assertPreviewPayload(offer);
}

function assertPreviewSubscribeOffer(offer: OfferSummary): void {
  if (!offer.accessModes.includes("subscribe")) {
    throw new Error(`Offer ${offer.domainId}/${offer.offerId} does not advertise Subscribe`);
  }
  assertPreviewPayload(offer);
}

function assertPreviewPayload(offer: OfferSummary): void {
  if (!isPreviewOffer(offer)) {
    throw new Error(`Offer ${offer.domainId}/${offer.offerId} is not a preview offer`);
  }
}

function requirePreviewOffer(offer: OfferSummary | undefined): OfferSummary {
  if (!offer) {
    throw new Error("No preview offer is available");
  }
  return offer;
}
