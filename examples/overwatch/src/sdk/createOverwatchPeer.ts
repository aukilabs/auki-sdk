import initAukiDomain, { AukiBrowserDomainPeer } from "@aukilabs/auki-domain";
import initAukiNetwork, {
  createAukiNetworkPeer,
  DiscoveryDirectoryClient,
} from "@aukilabs/auki-network";

import type { OverwatchPeer } from "./contract";

let initialized = false;

export async function createOverwatchPeer(): Promise<OverwatchPeer> {
  if (!initialized) {
    await initAukiNetwork();
    await initAukiDomain();
    initialized = true;
  }
  const walletSeed = loadOrMintWalletSeed();
  const networkPeer = await createAukiNetworkPeer({ walletSeed });
  return new AukiBrowserDomainPeer({
    networkPeer,
    discoveryFactory: (url: string) => new DiscoveryDirectoryClient(url),
    appId: "overwatch",
    displayName: browserDisplayName(networkPeer.peerId),
  }) as OverwatchPeer;
}

export function loadOrMintWalletSeed(): Uint8Array {
  const key = "auki:overwatch:wallet-seed:v1";
  const existing = globalThis.localStorage?.getItem(key);
  if (existing) {
    return Uint8Array.from(existing.match(/../g) ?? [], (byte) => Number.parseInt(byte, 16));
  }
  const seed = globalThis.crypto.getRandomValues(new Uint8Array(32));
  globalThis.localStorage?.setItem(
    key,
    Array.from(seed, (byte) => byte.toString(16).padStart(2, "0")).join(""),
  );
  return seed;
}

function browserDisplayName(peerId: string): string {
  return `Browser ${peerId.slice(-6)}`;
}
