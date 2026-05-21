import { describe, expect, it } from "vitest";
import { loadOrCreateSeed, memorySeedStore, shortPeerId } from "./identity.js";

describe("browser identity helpers", () => {
  it("persists generated seed through the provided store", async () => {
    const store = memorySeedStore();
    const first = await loadOrCreateSeed(store, () => new Uint8Array(32).fill(7));
    const second = await loadOrCreateSeed(store, () => new Uint8Array(32).fill(9));

    expect(Array.from(first)).toEqual(new Array(32).fill(7));
    expect(Array.from(second)).toEqual(new Array(32).fill(7));
  });

  it("rejects stored seeds that are not 32 bytes", async () => {
    const store = memorySeedStore(new Uint8Array([1, 2, 3]));

    await expect(loadOrCreateSeed(store, () => new Uint8Array(32))).rejects.toThrow(
      "Stored browser peer seed must be 32 bytes",
    );
  });

  it("formats short peer ids from the last six characters", () => {
    expect(shortPeerId("12D3KooWAvnEo4RaYZtqt2w83qzmQ7WVW2HhN2cay95EXAiVKcar")).toBe(
      "iVKcar",
    );
  });
});
