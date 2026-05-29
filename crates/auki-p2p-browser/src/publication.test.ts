import { describe, expect, it } from "vitest";
import {
  createPublicationSpatialMessage,
  LatestPublishedByteSource,
  openBackpressuredByteSource,
  type LocalOfferPublication,
} from "./publication.js";

const DOMAIN_ID = "noEv5Zu7UvR7qx9ooyAHd407PWcp8nUQLNRxrnd1ZRs";

describe("browser publication sources", () => {
  it("replays the latest frame and streams updates", async () => {
    const source = new LatestPublishedByteSource();
    expect(source.latest()).toBeUndefined();
    expect(source.isClosed()).toBe(false);

    expect(
      source.publish({
        bytes: new Uint8Array([1]),
        sequence: 4,
        generatedAt: "2026-05-29T00:00:00Z",
      }),
    ).toBe(true);
    expect(source.latest()).toEqual({
      bytes: new Uint8Array([1]),
      sequence: 4,
      generatedAt: "2026-05-29T00:00:00Z",
    });
    expect(source.latestBytes()).toEqual(new Uint8Array([1]));

    const iterator = source.stream()[Symbol.asyncIterator]();
    await expect(iterator.next()).resolves.toEqual({
      done: false,
      value: {
        bytes: new Uint8Array([1]),
        sequence: 4,
        generatedAt: "2026-05-29T00:00:00Z",
      },
    });

    const next = iterator.next();
    source.publish({
      bytes: new Uint8Array([2]),
      sequence: 5,
      generatedAt: "2026-05-29T00:00:01Z",
    });
    await expect(next).resolves.toEqual({
      done: false,
      value: {
        bytes: new Uint8Array([2]),
        sequence: 5,
        generatedAt: "2026-05-29T00:00:01Z",
      },
    });

    source.close();
    expect(source.isClosed()).toBe(true);
    expect(source.publish(new Uint8Array([3]))).toBe(false);
    await expect(iterator.next()).resolves.toEqual({ done: true, value: undefined });
  });

  it("preserves producer frame sequence and generated timestamp in spatial messages", async () => {
    const publication = testPublication();

    const first = await createPublicationSpatialMessage(publication, {
      bytes: new Uint8Array([1, 2, 3]),
      sequence: 42,
      generatedAt: "2026-05-29T00:00:00Z",
    });
    const second = await createPublicationSpatialMessage(
      publication,
      new Uint8Array([4, 5, 6]),
    );

    expect(first).toMatchObject({
      type: "auki.spatial_message.v1",
      domain_id: DOMAIN_ID,
      offer_id: "browser-preview",
      sequence: "42",
      generated_at: "2026-05-29T00:00:00Z",
      payload: {
        type: "example.bytes.v1",
        bytes: "AQID",
      },
    });
    expect(second).toMatchObject({
      sequence: "43",
      payload: {
        bytes: "BAUG",
      },
    });
  });

  it("keeps only the newest queued frame with LatestOnly backpressure", async () => {
    const iterator = openBackpressuredByteSource(
      [new Uint8Array([1]), new Uint8Array([2]), new Uint8Array([3])],
      { kind: "LatestOnly" },
    );

    await flushMicrotasks();

    await expect(iterator.next()).resolves.toEqual({
      done: false,
      value: { kind: "chunk", chunk: new Uint8Array([3]) },
    });
    await expect(iterator.next()).resolves.toEqual({
      done: false,
      value: { kind: "complete" },
    });
    await expect(iterator.next()).resolves.toEqual({ done: true, value: undefined });
  });

  it("preserves source order with bounded backpressure", async () => {
    const iterator = openBackpressuredByteSource(
      [new Uint8Array([1]), new Uint8Array([2]), new Uint8Array([3])],
      { kind: "Bounded", capacity: 2 },
    );

    await expect(iterator.next()).resolves.toEqual({
      done: false,
      value: { kind: "chunk", chunk: new Uint8Array([1]) },
    });
    await expect(iterator.next()).resolves.toEqual({
      done: false,
      value: { kind: "chunk", chunk: new Uint8Array([2]) },
    });
    await expect(iterator.next()).resolves.toEqual({
      done: false,
      value: { kind: "chunk", chunk: new Uint8Array([3]) },
    });
    await expect(iterator.next()).resolves.toEqual({
      done: false,
      value: { kind: "complete" },
    });
  });

  it("closes the queued stream when CloseOnFull capacity is exceeded", async () => {
    const iterator = openBackpressuredByteSource(
      [new Uint8Array([1]), new Uint8Array([2])],
      { kind: "CloseOnFull", capacity: 1 },
    );

    await flushMicrotasks();

    await expect(iterator.next()).resolves.toEqual({
      done: false,
      value: { kind: "close_for_backpressure" },
    });
    await expect(iterator.next()).resolves.toEqual({ done: true, value: undefined });
  });
});

function testPublication(): LocalOfferPublication {
  const raw = {
    offer_id: "browser-preview",
    domain_id: DOMAIN_ID,
    kind: "example.bytes",
    status: "available",
    access_modes: ["get", "subscribe"],
    payload: {
      type: "example.bytes.v1",
      encoding: "binary",
      media_type: "application/octet-stream",
      schema_version: "1",
    },
    registry_refs: [],
  };
  return {
    source: [],
    offer: {
      peerId: "browser-peer",
      domainId: DOMAIN_ID,
      offerId: "browser-preview",
      kind: "example.bytes",
      payloadType: "example.bytes.v1",
      accessModes: ["get", "subscribe"],
      raw,
    },
    stopped: false,
    nextSequence: 0n,
    backpressurePolicy: { kind: "Bounded", capacity: 1024 },
  };
}

async function flushMicrotasks(count = 8): Promise<void> {
  for (let index = 0; index < count; index += 1) {
    await Promise.resolve();
  }
}
