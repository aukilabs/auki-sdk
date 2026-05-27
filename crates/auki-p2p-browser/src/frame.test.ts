import { describe, expect, it } from "vitest";
import { FrameError, decodeJsonFrame, decodeLength, encodeJsonFrame, encodeLength } from "./frame.js";

describe("v1 JSON frames", () => {
  it("encodes and decodes unsigned LEB128 lengths like auki-protocol", () => {
    expect(Array.from(encodeLength(0))).toEqual([0x00]);
    expect(Array.from(encodeLength(127))).toEqual([0x7f]);
    expect(Array.from(encodeLength(128))).toEqual([0x80, 0x01]);
    expect(decodeLength(new Uint8Array([0x80, 0x80, 0x01]), 20_000)).toEqual([16_384n, 3]);
  });

  it("rejects non-minimal length prefixes", () => {
    expect(() => decodeLength(new Uint8Array([0x80, 0x00]), 20_000)).toThrow(FrameError);
    try {
      decodeLength(new Uint8Array([0x80, 0x00]), 20_000);
    } catch (error) {
      expect(error).toMatchObject({ code: "non_minimal_length" });
    }
  });

  it("encodes and decodes compact JSON object frames", () => {
    const frame = encodeJsonFrame({ type: "auki.test", ok: true }, 1024);
    const decoded = decodeJsonFrame(frame, 1024);

    expect(decoded.value).toEqual({ type: "auki.test", ok: true });
    expect(decoded.consumed).toBe(frame.byteLength);
  });

  it("rejects oversized and non-object frames", () => {
    expect(() => encodeJsonFrame({ payload: "too-big" }, 4)).toThrow(FrameError);
    expect(() => decodeJsonFrame(new Uint8Array([0x04, 0x6e, 0x75, 0x6c, 0x6c]), 1024)).toThrow(
      FrameError,
    );
  });
});
