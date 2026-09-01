import {
  AukiBlobClient,
  AukiCatalogClient,
  AukiInfoClient,
  AukiPeer,
  AukiRegistryClient,
  type AukiBlobReceipt,
  type AukiCatalogMapsResponse,
  type AukiCatalogResourcesResponse,
  type AukiExactTarget,
  type AukiParticipantInfo,
  type AukiRegistryEntry,
  type AukiRegistryListEntry,
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

void [info, resources, maps, registryList, registryEntry, blob];
