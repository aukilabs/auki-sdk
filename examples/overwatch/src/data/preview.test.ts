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

  it("renders encoded native camera frames as JPEG blobs", () => {
    const jpeg = new Uint8Array([0xff, 0xd8, 0xff, 0xd9]);
    const cameraFrame = new Uint8Array([0x12, 0x04, ...jpeg]);
    const revokeSpy = vi.fn();
    vi.stubGlobal("URL", {
      ...URL,
      createObjectURL: vi.fn((blob: Blob) => `blob:${blob.size}:${blob.type}`),
      revokeObjectURL: revokeSpy,
    });
    subscribeRuntimeStream.mockImplementationOnce((_spec, cb) => {
      cb({
        descriptor: { sensor_hash: "hash", clock_id: "clock" },
        payload: cameraFrame,
        sensorKind: "camera",
        seq: 8,
        timestamp_ns: 13,
        receivedAt: 11,
        receivedAtWallMs: 21,
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

    expect(frames.at(-1)).toMatchObject({
      url: "blob:4:image/jpeg",
      bytes: 4,
      seq: 8,
      timestamp_ns: 13,
    });

    dispose();
    expect(revokeSpy).toHaveBeenCalledWith("blob:4:image/jpeg");
  });

  it("preserves raw JPEG payloads from camera sensors", () => {
    const jpeg = new Uint8Array([0xff, 0xd8, 0xff, 0xd9]);
    const revokeSpy = vi.fn();
    vi.stubGlobal("URL", {
      ...URL,
      createObjectURL: vi.fn((blob: Blob) => `blob:${blob.size}:${blob.type}`),
      revokeObjectURL: revokeSpy,
    });
    subscribeRuntimeStream.mockImplementationOnce((_spec, cb) => {
      cb({
        descriptor: { sensor_hash: "hash", clock_id: "clock" },
        payload: jpeg,
        sensorKind: "camera",
        seq: 9,
        timestamp_ns: 14,
        receivedAt: 12,
        receivedAtWallMs: 22,
      });
      return vi.fn();
    });

    const frames: unknown[] = [];
    const dispose = subscribePreview(
      {
        peer_id: "12D3KooWCameraPeer",
        sensor_id: "overwatch/browser/demo-camera",
      },
      (frame) => frames.push(frame),
    );

    expect(frames.at(-1)).toMatchObject({
      url: "blob:4:image/jpeg",
      bytes: 4,
      seq: 9,
      timestamp_ns: 14,
    });

    dispose();
    expect(revokeSpy).toHaveBeenCalledWith("blob:4:image/jpeg");
  });

  it("skips invalid camera payloads without stopping later previews", () => {
    const jpeg = new Uint8Array([0xff, 0xd8, 0xff, 0xd9]);
    const revokeSpy = vi.fn();
    const createObjectURL = vi.fn((blob: Blob) => `blob:${blob.size}:${blob.type}`);
    vi.stubGlobal("URL", {
      ...URL,
      createObjectURL,
      revokeObjectURL: revokeSpy,
    });
    subscribeRuntimeStream.mockImplementationOnce((_spec, cb) => {
      cb({
        descriptor: { sensor_hash: "hash", clock_id: "clock" },
        payload: new Uint8Array([0x12]),
        sensorKind: "camera",
        seq: 10,
        timestamp_ns: 15,
        receivedAt: 13,
        receivedAtWallMs: 23,
      });
      cb({
        descriptor: { sensor_hash: "hash", clock_id: "clock" },
        payload: new Uint8Array([0x12, 0x00]),
        sensorKind: "camera",
        seq: 11,
        timestamp_ns: 16,
        receivedAt: 14,
        receivedAtWallMs: 24,
      });
      cb({
        descriptor: { sensor_hash: "hash", clock_id: "clock" },
        payload: jpeg,
        sensorKind: "camera",
        seq: 12,
        timestamp_ns: 17,
        receivedAt: 15,
        receivedAtWallMs: 25,
      });
      return vi.fn();
    });

    const frames: unknown[] = [];
    let dispose: (() => void) | undefined;
    expect(() => {
      dispose = subscribePreview(
        {
          peer_id: "12D3KooWCameraPeer",
          sensor_id: "K1-WALK01/head_left_cam",
        },
        (frame) => frames.push(frame),
      );
    }).not.toThrow();

    expect(createObjectURL).toHaveBeenCalledOnce();
    expect(frames).toHaveLength(1);
    expect(frames.at(-1)).toMatchObject({
      url: "blob:4:image/jpeg",
      bytes: 4,
      seq: 12,
      timestamp_ns: 17,
    });

    dispose?.();
    expect(revokeSpy).toHaveBeenCalledWith("blob:4:image/jpeg");
  });
});
