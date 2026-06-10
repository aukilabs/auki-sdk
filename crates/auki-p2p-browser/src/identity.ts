import { generateKeyPairFromSeed } from "@libp2p/crypto/keys";
import { xxhash128 } from "hash-wasm";

export type SeedStore = {
  load(): Promise<Uint8Array | null>;
  save(seed: Uint8Array): Promise<void>;
};

export type IndexedDbSeedStoreOptions = {
  databaseName?: string;
  storeName?: string;
  key?: string;
  indexedDB?: IDBFactory;
};

const DEFAULT_DATABASE_NAME = "auki-p2p-browser";
const DEFAULT_STORE_NAME = "identity";
const DEFAULT_SEED_KEY = "wallet-seed";

export async function loadOrCreateSeed(
  store: SeedStore,
  randomSeed: () => Uint8Array = cryptoRandomSeed,
): Promise<Uint8Array> {
  const existing = await store.load();
  if (existing) {
    assertSeedLength(existing, "Stored browser peer seed");
    return new Uint8Array(existing);
  }

  const seed = randomSeed();
  assertSeedLength(seed, "Generated browser peer seed");
  await store.save(seed);
  return new Uint8Array(seed);
}

export function memorySeedStore(initial: Uint8Array | null = null): SeedStore {
  let seed = initial ? new Uint8Array(initial) : null;
  return {
    async load() {
      return seed ? new Uint8Array(seed) : null;
    },
    async save(next) {
      assertSeedLength(next, "Browser peer seed");
      seed = new Uint8Array(next);
    },
  };
}

export function indexedDbSeedStore(options: IndexedDbSeedStoreOptions = {}): SeedStore {
  const databaseName = options.databaseName ?? DEFAULT_DATABASE_NAME;
  const storeName = options.storeName ?? DEFAULT_STORE_NAME;
  const key = options.key ?? DEFAULT_SEED_KEY;
  const idb = options.indexedDB ?? globalThis.indexedDB;
  if (!idb) {
    throw new Error("IndexedDB is not available in this browser context");
  }

  return {
    async load() {
      const db = await openDatabase(idb, databaseName, storeName);
      try {
        const value = await requestResult<Uint8Array | ArrayBuffer | number[] | undefined>(
          db.transaction(storeName, "readonly").objectStore(storeName).get(key),
        );
        if (value === undefined) return null;
        return bytesFromStoredSeed(value);
      } finally {
        db.close();
      }
    },
    async save(seed) {
      assertSeedLength(seed, "Browser peer seed");
      const db = await openDatabase(idb, databaseName, storeName);
      try {
        await requestResult(
          db
            .transaction(storeName, "readwrite")
            .objectStore(storeName)
            .put(new Uint8Array(seed), key),
        );
      } finally {
        db.close();
      }
    },
  };
}

export async function peerIdFromSeed(seed: Uint8Array): Promise<string> {
  const peerSeed = await derivePeerSeed(seed);
  const key = await generateKeyPairFromSeed("Ed25519", peerSeed);
  return key.publicKey.toString();
}

export async function derivePeerSeed(seed: Uint8Array, label = "peer/v1"): Promise<Uint8Array> {
  assertSeedLength(seed, "Browser peer wallet seed");
  const encoder = new TextEncoder();
  const firstInput = concatBytes(seed, encoder.encode(label));
  const firstHalfHex = await xxhash128(firstInput);
  const secondHalfHex = await xxhash128(concatBytes(firstInput, encoder.encode("/expand")));
  return hexToBytes(`${firstHalfHex}${secondHalfHex}`);
}

function cryptoRandomSeed(): Uint8Array {
  const seed = new Uint8Array(32);
  globalThis.crypto.getRandomValues(seed);
  return seed;
}

function assertSeedLength(seed: Uint8Array, label: string): void {
  if (seed.byteLength !== 32) {
    throw new Error(`${label} must be 32 bytes`);
  }
}

function openDatabase(
  idb: IDBFactory,
  databaseName: string,
  storeName: string,
): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const request = idb.open(databaseName, 1);
    request.onupgradeneeded = () => {
      const db = request.result;
      if (!db.objectStoreNames.contains(storeName)) {
        db.createObjectStore(storeName);
      }
    };
    request.onerror = () => reject(request.error ?? new Error("Opening IndexedDB failed"));
    request.onsuccess = () => resolve(request.result);
  });
}

function requestResult<T = unknown>(request: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    request.onerror = () => reject(request.error ?? new Error("IndexedDB request failed"));
    request.onsuccess = () => resolve(request.result);
  });
}

function bytesFromStoredSeed(value: Uint8Array | ArrayBuffer | number[]): Uint8Array {
  if (value instanceof Uint8Array) return new Uint8Array(value);
  if (value instanceof ArrayBuffer) return new Uint8Array(value);
  if (Array.isArray(value)) return new Uint8Array(value);
  throw new Error("Stored browser peer seed must be bytes");
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
  for (let index = 0; index < out.length; index += 1) {
    out[index] = Number.parseInt(hex.slice(index * 2, index * 2 + 2), 16);
  }
  return out;
}
