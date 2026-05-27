import { describe, expect, it } from "vitest";
import { stat } from "node:fs/promises";
import { resolve } from "node:path";

describe("Park brand assets", () => {
  it.each(["auki-monogram-white.svg", "auki-wordmark-white.svg"])(
    "serves /brand/%s from Vite public assets",
    async (fileName) => {
      const asset = resolve(__dirname, "..", "public", "brand", fileName);
      const result = await stat(asset);

      expect(result.isFile()).toBe(true);
    },
  );
});
