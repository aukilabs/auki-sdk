import { describe, expect, it, vi } from "vitest";

import { createCameraJpegSource, WEBCAM_JPEG_DEFAULTS } from "./cameraSource";

describe("WEBCAM_JPEG_DEFAULTS", () => {
  it("targets 30 FPS with lower JPEG quality", () => {
    expect(WEBCAM_JPEG_DEFAULTS.intervalMs).toBe(1000 / 30);
    expect(WEBCAM_JPEG_DEFAULTS.quality).toBe(0.5);
    expect(WEBCAM_JPEG_DEFAULTS.facingMode).toBe("environment");
  });
});

describe("createCameraJpegSource", () => {
  it("yields SDK stream entries from captured JPEG bytes", async () => {
    const source = createCameraJpegSource({
      intervalMs: 10,
      captureFrame: vi
        .fn()
        .mockResolvedValueOnce(new Uint8Array([1, 2, 3]))
        .mockResolvedValueOnce(new Uint8Array([4, 5])),
      sleep: async () => {},
      nowNs: () => 123,
    });
    const iterator = source[Symbol.asyncIterator]();

    await expect(iterator.next()).resolves.toEqual({
      done: false,
      value: { timestamp_ns: 123, seq: 0, payload: new Uint8Array([1, 2, 3]) },
    });
    await expect(iterator.next()).resolves.toEqual({
      done: false,
      value: { timestamp_ns: 123, seq: 1, payload: new Uint8Array([4, 5]) },
    });
  });
});
