import { afterEach, describe, expect, it, vi } from "vitest";

const { subscribeRuntimeStream, decodePointCloud2 } = vi.hoisted(() => ({
  subscribeRuntimeStream: vi.fn(),
  decodePointCloud2: vi.fn(() => ({
    frameId: "test_frame",
    height: 1,
    width: 1,
    positions: new Float32Array([1, 2, 3]),
    pointCount: 1,
  })),
}));

vi.mock("../sdk/streamHub", () => ({
  subscribeRuntimeStream,
}));

vi.mock("./cdrPointCloud", () => ({
  decodePointCloud2,
}));

import { subscribePointCloud } from "./pointcloudPreview";

describe("subscribePointCloud", () => {
  afterEach(() => {
    vi.clearAllMocks();
    vi.unstubAllGlobals();
  });

  it("subscribes through the SDK stream hub and never fetches latest.cdr", () => {
    const fetchSpy = vi.fn();
    vi.stubGlobal("fetch", fetchSpy);
    subscribeRuntimeStream.mockImplementationOnce((_spec, cb) => {
      cb({
        descriptor: null,
        payload: new Uint8Array([1, 2, 3, 4]),
        seq: 9,
        timestamp_ns: 24,
        receivedAt: 10,
        receivedAtWallMs: 20,
      });
      return vi.fn();
    });

    const frames: unknown[] = [];
    const dispose = subscribePointCloud(
      {
        peer_id: "12D3KooWCloudPeer",
        sensor_id: "K1-WALK01/pointcloud",
      },
      (frame) => frames.push(frame),
    );

    expect(subscribeRuntimeStream).toHaveBeenCalledWith(
      {
        peer_id: "12D3KooWCloudPeer",
        sensor_id: "K1-WALK01/pointcloud",
      },
      expect.any(Function),
    );
    expect(frames.at(-1)).toMatchObject({
      frameId: "test_frame",
      seq: 9,
      timestamp_ns: 24,
      pointCount: 1,
    });
    expect(fetchSpy).not.toHaveBeenCalled();

    dispose();
  });
});
