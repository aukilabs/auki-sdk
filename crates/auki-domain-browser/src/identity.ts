import { generateKeyPairFromSeed } from "@libp2p/crypto/keys";
import { xxhash128 } from "hash-wasm";

export type SeedStore = {
  load(): Promise<Uint8Array | null>;
  save(seed: Uint8Array): Promise<void>;
};

export async function loadOrCreateSeed(
  store: SeedStore,
  randomSeed: () => Uint8Array = cryptoRandomSeed,
): Promise<Uint8Array> {
  const existing = await store.load();
  if (existing) {
    if (existing.byteLength !== 32) {
      throw new Error("Stored browser peer seed must be 32 bytes");
    }
    return existing;
  }

  const seed = randomSeed();
  if (seed.byteLength !== 32) {
    throw new Error("Generated browser peer seed must be 32 bytes");
  }
  await store.save(seed);
  return seed;
}

export function memorySeedStore(initial: Uint8Array | null = null): SeedStore {
  let seed = initial;
  return {
    async load() {
      return seed ? new Uint8Array(seed) : null;
    },
    async save(next) {
      seed = new Uint8Array(next);
    },
  };
}

export function shortPeerId(peerId: string): string {
  return peerId.slice(-6);
}

export async function peerIdFromSeed(seed: Uint8Array): Promise<string> {
  const peerSeed = await derivePeerSeed(seed);
  const key = await generateKeyPairFromSeed("Ed25519", peerSeed);
  return key.publicKey.toString();
}

export async function derivePeerSeed(seed: Uint8Array, label = "peer/v1"): Promise<Uint8Array> {
  if (seed.byteLength !== 32) {
    throw new Error("Browser peer wallet seed must be 32 bytes");
  }
  const labelBytes = new TextEncoder().encode(label);
  const firstInput = concatBytes(seed, labelBytes);
  const firstHalfHex = await xxhash128(firstInput);
  const secondHalfHex = await xxhash128(concatBytes(firstInput, new TextEncoder().encode("/expand")));
  return hexToBytes(`${firstHalfHex}${secondHalfHex}`);
}

function cryptoRandomSeed(): Uint8Array {
  const seed = new Uint8Array(32);
  globalThis.crypto.getRandomValues(seed);
  return seed;
}

function concatBytes(...parts: Uint8Array[]): Uint8Array {
  const out = new Uint8Array(parts.reduce((sum, part) => sum + part.byteLength, 0));
  let offset = 0;
  for (const part of parts) {
    out.set(part, offset);
    offset += part.byteLength;
  }
  return out;
}

function hexToBytes(hex: string): Uint8Array {
  if (!/^[0-9a-f]+$/.test(hex) || hex.length % 2 !== 0) {
    throw new Error("Expected lowercase even-length hex");
  }
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i += 1) {
    out[i] = Number.parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  }
  return out;
}
