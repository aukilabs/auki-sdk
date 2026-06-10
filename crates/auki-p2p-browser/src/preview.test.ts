import { describe, expect, it } from "vitest";
import type { PublishOfferOptions } from "./publication.js";
import {
  decodeBase64UrlBytes,
  findPreviewOffer,
  PREVIEW_ACCESS_MODES,
  PREVIEW_OFFER_KIND,
  PREVIEW_PAYLOAD_ENCODING,
  PREVIEW_PAYLOAD_MEDIA_TYPE,
  PREVIEW_PAYLOAD_SCHEMA_VERSION,
  PREVIEW_PAYLOAD_TYPE,
  previewFrameFromMessage,
  previewFrameBytes,
  previewPayloadDescriptor,
  publishGeneratedPreview,
  publishPreviewOffer,
  type OfferPublisher,
} from "./preview.js";
import {
  AukiPreviewBrowserSession,
  getPreviewSnapshot,
  openPreviewSubscription,
  subscribePreview,
} from "./preview_client.js";
import type { AukiBrowserBootstrapRecord } from "./bootstrap.js";
import type {
  AukiBrowserPeer,
  AukiBrowserSubscription,
  OfferSummary,
  SpatialMessage,
} from "./peer.js";

describe("preview offer profile", () => {
  it("builds the shared JPEG payload descriptor", () => {
    expect(previewPayloadDescriptor()).toEqual({
      type: PREVIEW_PAYLOAD_TYPE,
      encoding: PREVIEW_PAYLOAD_ENCODING,
      media_type: PREVIEW_PAYLOAD_MEDIA_TYPE,
      schema_version: PREVIEW_PAYLOAD_SCHEMA_VERSION,
    });
  });

  it("publishes through the generic offer API", async () => {
    const handle = { stop: async () => undefined };
    let published: PublishOfferOptions | undefined;
    const publisher: OfferPublisher = {
      publishOffer: async (options) => {
        published = options;
        return handle;
      },
    };
    const source: Uint8Array[] = [new Uint8Array([0xff, 0xd8, 0xff, 0xd9])];

    await expect(
      publishPreviewOffer(publisher, source, {
        domainId: "domain-id",
        offerId: "preview-main",
        displayName: "Preview Main",
        metadata: { source: "generated" },
      }),
    ).resolves.toBe(handle);

    expect(published).toEqual({
      source,
      domainId: "domain-id",
      offerId: "preview-main",
      kind: PREVIEW_OFFER_KIND,
      payload: previewPayloadDescriptor(),
      accessModes: PREVIEW_ACCESS_MODES,
      backpressurePolicy: { kind: "LatestOnly" },
      displayName: "Preview Main",
      metadata: { source: "generated" },
    });
  });

  it("keeps the generated-preview helper as an alias over the same profile", async () => {
    let published: PublishOfferOptions | undefined;
    const publisher: OfferPublisher = {
      publishOffer: async (options) => {
        published = options;
        return { stop: async () => undefined };
      },
    };

    await publishGeneratedPreview(publisher, [], {
      domainId: "domain-id",
      offerId: "preview-main",
    });

    expect(published?.kind).toBe(PREVIEW_OFFER_KIND);
    expect(published?.payload).toEqual(previewPayloadDescriptor());
    expect(published?.backpressurePolicy).toEqual({ kind: "LatestOnly" });
  });

  it("allows preview publishers to override the Subscribe backpressure policy", async () => {
    let published: PublishOfferOptions | undefined;
    const publisher: OfferPublisher = {
      publishOffer: async (options) => {
        published = options;
        return { stop: async () => undefined };
      },
    };

    await publishPreviewOffer(publisher, [], {
      domainId: "domain-id",
      offerId: "preview-main",
      backpressurePolicy: { kind: "Bounded", capacity: 2 },
    });

    expect(published?.backpressurePolicy).toEqual({ kind: "Bounded", capacity: 2 });
  });

  it("finds preview offers by shared kind or payload type", () => {
    const offers: OfferSummary[] = [
      offerSummary({ offerId: "debug", kind: "debug", payloadType: "text/plain" }),
      offerSummary({ offerId: "preview-kind", kind: PREVIEW_OFFER_KIND }),
      offerSummary({ offerId: "preview-payload", payloadType: PREVIEW_PAYLOAD_TYPE }),
    ];

    expect(findPreviewOffer(offers)?.offerId).toBe("preview-kind");
    expect(findPreviewOffer(offers, (offer) => offer.offerId === "preview-payload")?.offerId).toBe(
      "preview-payload",
    );
  });

  it("decodes preview spatial messages into frame bytes", () => {
    const message = previewMessage("_9j_2Q", "12");

    const frame = previewFrameFromMessage(message);

    expect(Array.from(frame.bytes)).toEqual([255, 216, 255, 217]);
    expect(frame.sequence).toBe("12");
    expect(Array.from(previewFrameBytes(message))).toEqual([255, 216, 255, 217]);
    expect(Array.from(decodeBase64UrlBytes("AQID"))).toEqual([1, 2, 3]);
  });

  it("builds preview Get requests and returns decoded frames", async () => {
    let requested: unknown;
    const peer = {
      get: async (request: unknown) => {
        requested = request;
        return previewMessage("AQID", "7");
      },
    } as AukiBrowserPeer;

    const frame = await getPreviewSnapshot(peer, previewOffer("get-preview"));

    expect(requested).toEqual({
      peerId: "native-peer",
      domainId: "domain-id",
      offerId: "get-preview",
      params: undefined,
      acceptedPayloadTypes: [PREVIEW_PAYLOAD_TYPE],
      maxPayloadBytes: 1_048_576,
    });
    expect(Array.from(frame.bytes)).toEqual([1, 2, 3]);
    expect(frame.sequence).toBe("7");
  });

  it("wraps preview Subscribe messages as decoded frame streams", async () => {
    let stopped = 0;
    const subscription: AukiBrowserSubscription = {
      messages: (async function* () {
        yield previewMessage("AQID", "1");
        yield previewMessage("BAUG", "2");
      })(),
      stop: async () => {
        stopped += 1;
      },
    };
    const peer = {
      openSubscription: async () => subscription,
    } as AukiBrowserPeer;

    const frames = [];
    for await (const frame of subscribePreview(peer, previewOffer("stream-preview"))) {
      frames.push(Array.from(frame.bytes));
    }

    expect(frames).toEqual([
      [1, 2, 3],
      [4, 5, 6],
    ]);
    expect(stopped).toBe(1);

    const explicit = await openPreviewSubscription(peer, previewOffer("stream-preview"));
    await explicit.stop();
    expect(stopped).toBe(2);
  });

  it("keeps preview session methods on the high-level frame API", async () => {
    const offer = previewOffer("session-preview");
    const requested: unknown[] = [];
    const peer = {
      peerId: "browser-peer",
      listPeers: () => [
        {
          peerId: "native-peer",
          connected: true,
          dialAddresses: ["/memory"],
          observedAddresses: [],
          connectionPaths: [],
        },
      ],
      listOffers: async () => [offer],
      get: async (request: unknown) => {
        requested.push(request);
        return previewMessage("AQID", "3");
      },
      openSubscription: async (request: unknown) => {
        requested.push(request);
        return {
          messages: (async function* () {
            yield previewMessage("BAUG", "4");
          })(),
          stop: async () => undefined,
        };
      },
      stop: async () => undefined,
    } as unknown as AukiBrowserPeer;
    const session = new AukiPreviewBrowserSession(
      peer,
      bootstrapRecord("native-peer"),
      [],
      [offer],
      offer,
      {
        maxPayloadBytes: 123,
        maxMessageBytes: 456,
      },
    );

    const snapshot = await session.getSnapshot();
    const subscription = await session.openSubscription();
    const frames = [];
    for await (const frame of subscription.frames) {
      frames.push(Array.from(frame.bytes));
    }

    expect(Array.from(snapshot.bytes)).toEqual([1, 2, 3]);
    expect(frames).toEqual([[4, 5, 6]]);
    expect(requested).toEqual([
      expect.objectContaining({ offerId: "session-preview", maxPayloadBytes: 123 }),
      expect.objectContaining({ offerId: "session-preview", maxMessageBytes: 456 }),
    ]);
    await expect(session.refreshOffers()).resolves.toEqual([offer]);
    expect(session.peers).toEqual([
      {
        peerId: "native-peer",
        connected: true,
        dialAddresses: ["/memory"],
        observedAddresses: [],
        connectionPaths: [],
      },
    ]);
    expect(session.bootstrap.peerId).toBe("native-peer");
    expect(session.bootstraps.map((record) => record.peerId)).toEqual(["native-peer"]);
  });
});

function previewOffer(offerId: string): OfferSummary {
  return offerSummary({ offerId, accessModes: ["get", "subscribe"] });
}

function offerSummary(
  overrides: Partial<OfferSummary> = {},
): OfferSummary {
  return {
    peerId: "native-peer",
    domainId: "domain-id",
    offerId: "preview-main",
    kind: PREVIEW_OFFER_KIND,
    payloadType: PREVIEW_PAYLOAD_TYPE,
    accessModes: ["get", "subscribe"],
    ...overrides,
  };
}

function previewMessage(bytes: string, sequence: string): SpatialMessage {
  return {
    type: "auki.spatial_message.v1",
    domain_id: "domain-id",
    offer_id: "preview-main",
    sequence,
    generated_at: "2026-05-28T00:00:00Z",
    payload: {
      type: PREVIEW_PAYLOAD_TYPE,
      encoding: PREVIEW_PAYLOAD_ENCODING,
      bytes,
    },
  };
}

function bootstrapRecord(peerId: string): AukiBrowserBootstrapRecord {
  return {
    peerId,
    agentVersion: "test",
    directAddresses: ["/memory/native"],
    webrtcDirectAddresses: [],
    relayAddresses: [],
    relayServerAddresses: [],
    bootstrapAddresses: ["/memory/native"],
  };
}
