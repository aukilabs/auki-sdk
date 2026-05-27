import { afterEach, describe, expect, it, vi } from "vitest";

const { getStream } = vi.hoisted(() => ({
  getStream: vi.fn(),
}));

vi.mock("./runtime", () => ({
  sdkRuntime: {
    getStream,
  },
}));

import {
  getRuntimeStreamDescriptor,
  subscribeRuntimeStream,
} from "./streamHub";

describe("streamHub", () => {
  afterEach(() => {
    vi.clearAllMocks();
  });

  it("broadcasts SDK stream entries and stores the accept descriptor", async () => {
    const close = vi.fn();
    getStream.mockResolvedValueOnce(
      fakeStream([
        {
          accept: {
            sensor_id: "camera",
            sensor_hash: "camera-hash",
            clock_id: "clock",
            clock_hash: "clock-hash",
            frame_id: "frame",
            frame_hash: "frame-hash",
          },
        },
        {
          entry: {
            seq: 7,
            timestamp_ns: 42,
            payload: [255, 216, 255, 217],
          },
        },
        null,
      ], close),
    );

    const frames: unknown[] = [];
    const dispose = subscribeRuntimeStream(
      { peer_id: "peer-a", sensor_id: "camera" },
      (frame) => frames.push(frame),
    );

    await vi.waitFor(() => {
      expect(frames.at(-1)).toMatchObject({
        seq: 7,
        timestamp_ns: 42,
        payload: new Uint8Array([255, 216, 255, 217]),
      });
    });
    expect(getRuntimeStreamDescriptor({ peer_id: "peer-a", sensor_id: "camera" })).toMatchObject({
      sensor_id: "camera",
      sensor_hash: "camera-hash",
      clock_id: "clock",
    });

    dispose();
    expect(close).toHaveBeenCalled();
  });
});

function fakeStream(messages: unknown[], close: () => void) {
  return {
    async nextMessage() {
      return messages.shift();
    },
    close,
  };
}
