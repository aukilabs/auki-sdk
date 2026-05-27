import { afterEach, describe, expect, it, vi } from "vitest";

const { subscribeRuntimeStream, getRuntimeStreamState } = vi.hoisted(() => ({
  subscribeRuntimeStream: vi.fn(),
  getRuntimeStreamState: vi.fn(() => "live"),
}));

vi.mock("../sdk/streamHub", () => ({
  subscribeRuntimeStream,
  getRuntimeStreamState,
}));

import { getStreamState, subscribePreview } from "./preview";

describe("subscribePreview", () => {
  afterEach(() => {
    vi.clearAllMocks();
    vi.unstubAllGlobals();
  });

  it("subscribes through the SDK stream hub and never fetches latest.jpg", () => {
    const fetchSpy = vi.fn();
    const revokeSpy = vi.fn();
    vi.stubGlobal("fetch", fetchSpy);
    vi.stubGlobal("URL", {
      ...URL,
      createObjectURL: vi.fn(() => "blob:frame"),
      revokeObjectURL: revokeSpy,
    });
    subscribeRuntimeStream.mockImplementationOnce((_spec, cb) => {
      cb({
        descriptor: { sensor_hash: "hash", clock_id: "clock" },
        payload: new Uint8Array([255, 216, 255, 217]),
        seq: 4,
        timestamp_ns: 12,
        receivedAt: 10,
        receivedAtWallMs: 20,
      });
      return vi.fn();
    });

    const frames: unknown[] = [];
    const dispose = subscribePreview(
      {
        peer_id: "12D3KooWCameraPeer",
        sensor_id: "K1-WALK01/head_left_cam",
      },
      (frame) => frames.push(frame),
    );

    expect(subscribeRuntimeStream).toHaveBeenCalledWith(
      {
        peer_id: "12D3KooWCameraPeer",
        sensor_id: "K1-WALK01/head_left_cam",
      },
      expect.any(Function),
    );
    expect(frames.at(-1)).toMatchObject({
      url: "blob:frame",
      seq: 4,
      timestamp_ns: 12,
      sensorHash: "hash",
      clockId: "clock",
    });
    expect(fetchSpy).not.toHaveBeenCalled();
    expect(getStreamState({ peer_id: "p", sensor_id: "s" })).toBe("live");

    dispose();
    expect(revokeSpy).toHaveBeenCalledWith("blob:frame");
  });
});
