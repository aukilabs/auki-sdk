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
  AukiMessageReceiver,
  AukiMessageSender,
  AukiPeer,
  AukiPeerReachabilityMode,
  AukiUserSession,
  AukiRegistryClient,
  AukiRegistryEndpoint,
  AukiStreamClient,
  AukiStreamEndpoint,
  AukiStreamSubscription,
  decodeCameraFrameImage,
  encodeCameraFrameImage,
  prepareCatalogResources,
  prepareRegistryEntry,
  type AukiAuthenticatedPeer,
  type AukiBlobReceipt,
  type AukiBlobProvider,
  type AukiBlobProviderRequest,
  type AukiProvidedBlobChunk,
  type AukiCatalogMapsProvider,
  type AukiCatalogMapsResponse,
  type AukiCatalogResourcesProvider,
  type AukiCatalogResourcesRequest,
  type AukiCatalogResourcesResponse,
  type AukiExactTarget,
  type AukiDiscoveryCandidate,
  type AukiMessageChannelResource,
  type AukiMessageEvent,
  type AukiParticipantInfo,
  type AukiInfoProvider,
  type AukiRegistryEntry,
  type AukiRegistryEntryEnvelope,
  type AukiRegistryKind,
  type AukiRegistryListEntry,
  type AukiRegistryProvider,
  type AukiRegistryProviderRequest,
  type AukiRegistryProviderResponse,
  type AukiStreamDispatch,
  type AukiStreamProvider,
  type AukiStreamSourceItem,
  type AukiStreamEndReason,
  type AukiStreamEntry,
  type AukiStreamManifest,
  type AukiStreamNext,
  type AukiStreamPayloadKind,
  type AukiStreamReadFrom,
  type AukiStreamRequest,
} from "../pkg-test/auki_sdk_web.js";

declare const peer: AukiPeer;
declare const session: AukiUserSession;
const browserRoute: string | undefined = peer.wssRoute;
const nativeRoute: string | undefined = peer.tcpRoute;
const relayBacked: boolean = peer.relayBacked;
const defaultPeer: Promise<AukiPeer> = session.startPeer(
  "00000000-0000-0000-0000-000000000001",
);
const defaultDiscoveryPeer: Promise<AukiPeer> = session.startPeerWithDiscovery(
  "00000000-0000-0000-0000-000000000001",
  AukiDiscoveryMode.DiscoverOnly,
);
const outboundPeer: Promise<AukiPeer> = session.startPeer(
  "00000000-0000-0000-0000-000000000001",
  AukiPeerReachabilityMode.OutboundOnly,
);
const discoverOnlyPeer: Promise<AukiPeer> = session.startPeerWithDiscovery(
  "00000000-0000-0000-0000-000000000001",
  AukiDiscoveryMode.DiscoverOnly,
  AukiPeerReachabilityMode.OutboundOnly,
);
const advertisingPeer: Promise<AukiPeer> = session.startPeerWithDiscovery(
  "00000000-0000-0000-0000-000000000001",
  AukiDiscoveryMode.DiscoverAndAdvertise,
  AukiPeerReachabilityMode.RelayBacked,
);
const discoveredPeers: Promise<AukiDiscoveryCandidate[]> = peer.discover();
const discoveredEchoPeers: Promise<AukiDiscoveryCandidate[]> = peer.discoverProtocol(
  "/example/echo/1.0.0",
);
void [
  browserRoute,
  nativeRoute,
  relayBacked,
  defaultPeer,
  defaultDiscoveryPeer,
  outboundPeer,
  discoverOnlyPeer,
  advertisingPeer,
];

async function checkDiscoveryCandidate(): Promise<void> {
  const candidate = (await discoveredPeers)[0];
  if (candidate !== undefined) {
    const peerId: string = candidate.peerId;
    const routes: string[] = candidate.routes;
    const protocols: string[] = candidate.servedProtocols;
    const expiresAt: string = candidate.expiresAt;
    const source: string = candidate.source;
    void [peerId, routes, protocols, expiresAt, source];
  }
}

const target: AukiExactTarget = {
  peerId: "12D3KooW...",
  route: "/dns4/relay.example/tcp/443/wss/p2p/12D3KooW.../p2p-circuit/p2p/12D3KooW...",
};

const info: Promise<AukiParticipantInfo> = new AukiInfoClient(peer).fetchExact(target);
const infoProvider: AukiInfoProvider = () => ({
  app: "example",
  appVersion: "1.0.0",
  name: "browser",
  sessionId: "session",
  sessionClockId: "clock",
  sessionClockHash: "hash",
  sessionNowNs: 0n,
  peerId: peer.peerId,
  appInstance: "tab",
});
const infoEndpoint: AukiInfoEndpoint = AukiInfoEndpoint.mount(peer, infoProvider);
const infoClose: Promise<void> = infoEndpoint.close();

const resources: Promise<AukiCatalogResourcesResponse> = new AukiCatalogClient(
  peer,
).fetchResourcesExact(target, ["sensor_log", "message_channel"]);
const maps: Promise<AukiCatalogMapsResponse> = new AukiCatalogClient(peer).fetchMapsExact(target);
const catalogResourcesProvider: AukiCatalogResourcesProvider = (
  _requester,
  request: AukiCatalogResourcesRequest,
) => ({ resources: request.variants.length === 0 ? [] : [] });
const catalogMapsProvider: AukiCatalogMapsProvider = () => ({ resources: [] });
const catalogEndpoint: AukiCatalogEndpoint = AukiCatalogEndpoint.mount(
  peer,
  catalogResourcesProvider,
  catalogMapsProvider,
);
const catalogClose: Promise<void> = catalogEndpoint.close();

const registryList: Promise<AukiRegistryListEntry[]> = new AukiRegistryClient(peer).listExact(
  target,
  "sensor",
);
const registryEntry: Promise<AukiRegistryEntry> = new AukiRegistryClient(peer).fetchExact(
  target,
  "sensor",
  "camera",
  "0123456789abcdef0123456789abcdef",
);
async function checkRegistryEntryPreparation(): Promise<void> {
  const entry = await registryEntry;
  const envelope: AukiRegistryEntryEnvelope = prepareRegistryEntry("sensor", entry);
  const kind: AukiRegistryKind = envelope.kind;
  const id: string = envelope.id;
  const hash: string = envelope.hash;
  const canonicalJson: string = envelope.canonical_json;
  void [kind, id, hash, canonicalJson];
}
const registryProvider: AukiRegistryProvider = (
  _requester,
  request: AukiRegistryProviderRequest,
): AukiRegistryProviderResponse => ({
  op: "error",
  reason: `not configured: ${request.kind}`,
});
const registryEndpoint: AukiRegistryEndpoint = AukiRegistryEndpoint.mount(peer, registryProvider);
const registryClose: Promise<void> = registryEndpoint.close();

const blob: Promise<AukiBlobReceipt> = new AukiBlobClient(peer).fetchExact(
  target,
  "0".repeat(64),
);
const blobProvider: AukiBlobProvider = async (
  _requester,
  _request: AukiBlobProviderRequest,
): Promise<AukiProvidedBlobChunk | null> => null;
const blobEndpoint: AukiBlobEndpoint = AukiBlobEndpoint.mount(peer, blobProvider);
const blobClose: Promise<void> = blobEndpoint.close();

const channel: AukiMessageChannelResource = {
  variant: "message_channel",
  owner_peer_id: target.peerId,
  resource_id: "events",
  clock: {
    peer_id: target.peerId,
    id: "session/monotonic",
    hash: "0123456789abcdef0123456789abcdef",
  },
};
const messageEndpoint: AukiMessageEndpoint = AukiMessageEndpoint.mount(peer);
const messageReceiver: AukiMessageReceiver = messageEndpoint.declare(channel, 16);
const messageEvent: Promise<AukiMessageEvent | null> = messageReceiver.next();
const receiverClose: Promise<void> = messageReceiver.close();

async function checkMessageSender(): Promise<void> {
  const sender: AukiMessageSender = await new AukiMessageClient(peer).openExact(target, channel);
  const authenticated: AukiAuthenticatedPeer = sender.remotePeer;
  await sender.send("example.event", 1n, new Uint8Array([1, 2, 3]));
  await sender.close();
  void authenticated;
}

const streamKind: AukiStreamPayloadKind = "camera";
const encodedCameraFrame: Uint8Array = encodeCameraFrameImage(
  new Uint8Array([0xff, 0xd8, 0xff, 0xd9]),
);
const decodedCameraImage: Uint8Array = decodeCameraFrameImage(encodedCameraFrame);
const preparedCatalog: AukiCatalogResourcesResponse = prepareCatalogResources({ resources: [] });
const streamFrom: AukiStreamReadFrom = { kind: "latest" };
const streamRequest: AukiStreamRequest = {
  sourcePeerId: target.peerId,
  resourceId: "camera/front",
  from: streamFrom,
};

const streamManifest: AukiStreamManifest = {
  sensorId: "camera/front",
  sensorHash: "0123456789abcdef0123456789abcdef",
  clockPeerId: peer.peerId,
  clockId: "session/monotonic",
  clockHash: "0123456789abcdef0123456789abcdef",
  frameId: "camera/front",
  frameHash: "0123456789abcdef0123456789abcdef",
  resourceId: "camera/front",
  payload: "camera_frame",
  fromFrameId: "",
  fromFrameHash: "",
  toFrameId: "",
  toFrameHash: "",
  writerMode: "live",
  expectedRateHz: 30,
  mapPeerId: "",
  mapId: "",
  mapHash: "",
};

async function* cameraStreamSource(): AsyncIterable<AukiStreamSourceItem> {
  yield { timestampNs: 1n, payload: new Uint8Array() };
}

const streamProvider: AukiStreamProvider = (_requester, request): AukiStreamDispatch => {
  if (request.resourceId !== "camera/front") {
    return { kind: "decline", reason: { kind: "sensor_not_found" } };
  }
  return {
    kind: "accept",
    payloadKind: "camera",
    manifest: streamManifest,
    source: cameraStreamSource(),
  };
};
const streamEndpoint: AukiStreamEndpoint = AukiStreamEndpoint.mount(peer, streamProvider);
const streamClose: Promise<void> = streamEndpoint.close();

async function checkStreamConsumer(): Promise<void> {
  const subscription: AukiStreamSubscription = await new AukiStreamClient(peer).subscribeExact(
    target,
    streamKind,
    streamRequest,
  );
  const manifest: AukiStreamManifest = subscription.manifest;
  const next: AukiStreamNext | undefined = await subscription.next();
  if (next?.kind === "entry") {
    const entry: AukiStreamEntry = next.entry;
    const timestamp: bigint = entry.timestampNs;
    const payload: Uint8Array = entry.payload;
    void [timestamp, payload];
  } else if (next?.kind === "end") {
    const reason: AukiStreamEndReason = next.reason;
    void reason;
  }
  await subscription.cancel();
  void manifest;
}

void [
  info,
  infoEndpoint,
  infoClose,
  resources,
  maps,
  catalogEndpoint,
  catalogClose,
  registryList,
  registryEntry,
  checkRegistryEntryPreparation,
  registryEndpoint,
  registryClose,
  blob,
  blobEndpoint,
  blobClose,
  messageEndpoint,
  messageEvent,
  receiverClose,
  checkMessageSender,
  checkStreamConsumer,
  streamEndpoint,
  streamClose,
  encodedCameraFrame,
  decodedCameraImage,
  preparedCatalog,
  browserRoute,
  nativeRoute,
  discoverOnlyPeer,
  advertisingPeer,
  discoveredEchoPeers,
  checkDiscoveryCandidate,
];
