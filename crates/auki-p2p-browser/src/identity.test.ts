import { indexedDB } from "fake-indexeddb";
import { describe, expect, it } from "vitest";
import {
  derivePeerSeed,
  indexedDbSeedStore,
  loadOrCreateSeed,
  memorySeedStore,
  peerIdFromSeed,
} from "./identity.js";

describe("browser identity", () => {
  it("persists generated seeds through the configured store", async () => {
    const store = memorySeedStore();
    const first = await loadOrCreateSeed(store, () => new Uint8Array(32).fill(7));
    const second = await loadOrCreateSeed(store, () => new Uint8Array(32).fill(9));

    expect(Array.from(first)).toEqual(new Array(32).fill(7));
    expect(Array.from(second)).toEqual(new Array(32).fill(7));
  });

  it("persists browser peer seeds in IndexedDB", async () => {
    const options = {
      databaseName: `auki-p2p-browser-test-${crypto.randomUUID()}`,
      indexedDB,
    };
    const firstStore = indexedDbSeedStore(options);
    const secondStore = indexedDbSeedStore(options);

    await firstStore.save(new Uint8Array(32).fill(11));

    await expect(secondStore.load()).resolves.toEqual(new Uint8Array(32).fill(11));
  });

  it("rejects stored seeds that are not 32 bytes", async () => {
    const store = memorySeedStore(new Uint8Array([1, 2, 3]));

    await expect(loadOrCreateSeed(store, () => new Uint8Array(32))).rejects.toThrow(
      "Stored browser peer seed must be 32 bytes",
    );
  });

  it("matches the Rust Wallet -> peer/v1 -> PeerId vector", async () => {
    const seed = new Uint8Array(32).fill(3);
    const peerSeed = await derivePeerSeed(seed);

    expect(Array.from(peerSeed.slice(0, 4))).toEqual([0x82, 0x4f, 0xed, 0xdf]);
    await expect(peerIdFromSeed(seed)).resolves.toBe(
      "12D3KooWAvnEo4RaYZtqt2w83qzmQ7WVW2HhN2cay95EXAiVKcar",
    );
  });
});
