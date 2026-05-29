export type AukiBrowserBootstrapRecord = {
  peerId: string;
  agentVersion?: string;
  directAddresses: string[];
  webrtcDirectAddresses: string[];
  relayAddresses: string[];
  relayServerAddresses: string[];
  bootstrapAddresses: string[];
};

export type BootstrapAddressRole =
  | "webrtc_direct"
  | "relay_server"
  | "relay"
  | "direct"
  | "bootstrap";

export type BootstrapAddress = {
  address: string;
  roles: BootstrapAddressRole[];
};

const BROWSER_AGENT_VERSION = "auki-p2p-browser/0.0.0";

export function createLocalBootstrapRecord(
  peerId: string,
  addresses: string[],
  agentVersion = BROWSER_AGENT_VERSION,
): AukiBrowserBootstrapRecord {
  const peerAddresses = uniqueStrings(addresses).filter((address) =>
    isExportableBrowserBootstrapAddress(address, peerId),
  );
  const directAddresses = peerAddresses.filter((address) => !isRelayAddress(address));
  const webrtcDirectAddresses = directAddresses.filter(isWebrtcDirectAddress);
  const relayAddresses = peerAddresses.filter(isRelayAddress);
  const bootstrapAddresses = uniqueStrings([...directAddresses, ...relayAddresses]);

  if (bootstrapAddresses.length === 0) {
    throw new Error("Browser peer is not dialable yet");
  }

  return {
    peerId,
    agentVersion,
    directAddresses,
    webrtcDirectAddresses,
    relayAddresses,
    relayServerAddresses: [],
    bootstrapAddresses,
  };
}

export function parseBootstrapRecord(value: unknown): AukiBrowserBootstrapRecord {
  if (!value || typeof value !== "object") {
    throw new Error("Auki browser bootstrap record must be an object");
  }
  const object = value as Record<string, unknown>;
  const peerId = stringField(object, "peer_id") ?? stringField(object, "peerId");
  if (!peerId) {
    throw new Error("Auki browser bootstrap record is missing peer_id");
  }

  const directAddresses = stringArrayField(object, "direct_addresses", "directAddresses");
  const webrtcDirectAddresses = stringArrayField(
    object,
    "webrtc_direct_addresses",
    "webrtcDirectAddresses",
  );
  const relayAddresses = stringArrayField(object, "relay_addresses", "relayAddresses");
  const relayServerAddresses = stringArrayField(
    object,
    "relay_server_addresses",
    "relayServerAddresses",
  );
  const explicitBootstrapAddresses = optionalStringArrayField(
    object,
    "bootstrap_addresses",
    "bootstrapAddresses",
  );
  const bootstrapAddresses =
    explicitBootstrapAddresses ??
    uniqueStrings([
      ...directAddresses,
      ...webrtcDirectAddresses,
      ...relayAddresses,
      ...relayServerAddresses,
    ]);

  return {
    peerId,
    agentVersion: stringField(object, "agent_version") ?? stringField(object, "agentVersion"),
    directAddresses,
    webrtcDirectAddresses,
    relayAddresses,
    relayServerAddresses,
    bootstrapAddresses,
  };
}

export function parseBootstrapRecords(value: unknown): AukiBrowserBootstrapRecord[] {
  return (Array.isArray(value) ? value : [value]).map(parseBootstrapRecord);
}

export function bootstrapAddressBook(record: AukiBrowserBootstrapRecord): BootstrapAddress[] {
  const byAddress = new Map<string, Set<BootstrapAddressRole>>();
  addAddresses(byAddress, record.bootstrapAddresses, "bootstrap");
  addAddresses(byAddress, record.directAddresses, "direct");
  addAddresses(byAddress, record.webrtcDirectAddresses, "webrtc_direct");
  addAddresses(byAddress, record.relayAddresses, "relay");
  addAddresses(byAddress, record.relayServerAddresses, "relay_server");
  return Array.from(byAddress.entries()).map(([address, roles]) => ({
    address,
    roles: Array.from(roles),
  }));
}

export function preferredDialAddresses(record: AukiBrowserBootstrapRecord): string[] {
  return uniqueStrings([
    ...record.webrtcDirectAddresses,
    ...record.directAddresses,
    ...record.relayAddresses,
    ...record.bootstrapAddresses,
    ...record.relayServerAddresses,
  ]);
}

export function relayServerAddresses(record: AukiBrowserBootstrapRecord): string[] {
  return record.relayServerAddresses.slice();
}

export function isExportableBrowserBootstrapAddress(address: string, peerId: string): boolean {
  if (!addressTargetsPeer(address, peerId)) {
    return false;
  }
  if (isRelayAddress(address)) {
    return isStableRelayReservationAddress(address, peerId);
  }
  return isExportableDirectPeerAddress(address, peerId);
}

function addAddresses(
  byAddress: Map<string, Set<BootstrapAddressRole>>,
  addresses: string[],
  role: BootstrapAddressRole,
): void {
  for (const address of addresses) {
    const roles = byAddress.get(address) ?? new Set<BootstrapAddressRole>();
    roles.add(role);
    byAddress.set(address, roles);
  }
}

function stringField(object: Record<string, unknown>, key: string): string | undefined {
  const value = object[key];
  return typeof value === "string" && value.length > 0 ? value : undefined;
}

function stringArrayField(
  object: Record<string, unknown>,
  snakeKey: string,
  camelKey: string,
): string[] {
  const value = object[snakeKey] ?? object[camelKey];
  if (!Array.isArray(value)) {
    throw new Error(`Auki browser bootstrap record field ${snakeKey} must be a string array`);
  }
  return value.map((entry) => {
    if (typeof entry !== "string" || entry.length === 0) {
      throw new Error(`Auki browser bootstrap record field ${snakeKey} must contain only strings`);
    }
    return entry;
  });
}

function optionalStringArrayField(
  object: Record<string, unknown>,
  snakeKey: string,
  camelKey: string,
): string[] | undefined {
  if (object[snakeKey] === undefined && object[camelKey] === undefined) return undefined;
  return stringArrayField(object, snakeKey, camelKey);
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

function addressTargetsPeer(address: string, peerId: string): boolean {
  return address.endsWith(`/p2p/${peerId}`) || address.includes(`/p2p/${peerId}/`);
}

function isExportableDirectPeerAddress(address: string, peerId: string): boolean {
  return address.endsWith(`/p2p/${peerId}`) && !address.includes("/webrtc/");
}

function isRelayAddress(address: string): boolean {
  return address.includes("/p2p-circuit");
}

function isStableRelayReservationAddress(address: string, peerId: string): boolean {
  return address.endsWith(`/p2p/${peerId}`) && address.includes(`/p2p-circuit/p2p/${peerId}`);
}

function isWebrtcDirectAddress(address: string): boolean {
  return address.includes("/webrtc-direct");
}
