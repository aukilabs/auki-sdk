import { describe, expect, it } from "vitest";
import type { PublishOfferOptions } from "./publication.js";
import {
  PREVIEW_OFFER_KIND,
  PREVIEW_PAYLOAD_ENCODING,
  PREVIEW_PAYLOAD_MEDIA_TYPE,
  PREVIEW_PAYLOAD_SCHEMA_VERSION,
  PREVIEW_PAYLOAD_TYPE,
  previewPayloadDescriptor,
  publishGeneratedPreview,
  publishPreviewOffer,
  type OfferPublisher,
} from "./preview.js";

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
  });
});
