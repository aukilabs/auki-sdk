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

function cryptoRandomSeed(): Uint8Array {
  const seed = new Uint8Array(32);
  globalThis.crypto.getRandomValues(seed);
  return seed;
}
