import {
  AukiBlobClient,
  AukiCatalogClient,
  AukiInfoClient,
  AukiMessageClient,
  AukiMessageEndpoint,
  AukiMessageReceiver,
  AukiMessageSender,
  AukiPeer,
  AukiRegistryClient,
  AukiStreamClient,
  AukiStreamSubscription,
  type AukiAuthenticatedPeer,
  type AukiBlobReceipt,
  type AukiCatalogMapsResponse,
  type AukiCatalogResourcesResponse,
  type AukiExactTarget,
  type AukiMessageChannelResource,
  type AukiMessageEvent,
  type AukiParticipantInfo,
  type AukiRegistryEntry,
  type AukiRegistryListEntry,
  type AukiStreamEndReason,
  type AukiStreamEntry,
  type AukiStreamManifest,
  type AukiStreamNext,
  type AukiStreamPayloadKind,
  type AukiStreamReadFrom,
  type AukiStreamRequest,
} from "../pkg-test/auki_sdk_web.js";

declare const peer: AukiPeer;

const target: AukiExactTarget = {
  peerId: "12D3KooW...",
  route: "/dns4/relay.example/tcp/443/wss/p2p/12D3KooW.../p2p-circuit/p2p/12D3KooW...",
};

const info: Promise<AukiParticipantInfo> = new AukiInfoClient(peer).fetchExact(target);
const resources: Promise<AukiCatalogResourcesResponse> = new AukiCatalogClient(
  peer,
).fetchResourcesExact(target, ["sensor_log", "message_channel"]);
const maps: Promise<AukiCatalogMapsResponse> = new AukiCatalogClient(peer).fetchMapsExact(target);
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
const blob: Promise<AukiBlobReceipt> = new AukiBlobClient(peer).fetchExact(
  target,
  "0".repeat(64),
);

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
const streamFrom: AukiStreamReadFrom = { kind: "latest" };
const streamRequest: AukiStreamRequest = {
  sourcePeerId: target.peerId,
  resourceId: "camera/front",
  from: streamFrom,
};

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
  resources,
  maps,
  registryList,
  registryEntry,
  blob,
  messageEndpoint,
  messageEvent,
  receiverClose,
  checkMessageSender,
  checkStreamConsumer,
];
