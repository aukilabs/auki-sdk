import type { ByteSourceInput, PublicationHandle, PublishOfferOptions } from "./publication.js";

export const GENERATED_PREVIEW_OFFER_KIND = "auki.sensor.rgb_camera.preview";
export const GENERATED_PREVIEW_PAYLOAD_TYPE = "auki.camera.jpeg_frame.v1";

export type GeneratedPreviewOptions = {
  domainId: string;
  offerId: string;
  kind?: string;
  payloadType?: string;
  displayName?: string;
  metadata?: Record<string, unknown>;
};

export type OfferPublisher = {
  publishOffer(options: PublishOfferOptions): Promise<PublicationHandle>;
};

export function publishGeneratedPreview(
  peer: OfferPublisher,
  source: ByteSourceInput,
  options: GeneratedPreviewOptions,
): Promise<PublicationHandle> {
  return peer.publishOffer({
    source,
    domainId: options.domainId,
    offerId: options.offerId,
    kind: options.kind ?? GENERATED_PREVIEW_OFFER_KIND,
    payload: {
      type: options.payloadType ?? GENERATED_PREVIEW_PAYLOAD_TYPE,
      encoding: "binary",
      media_type: "image/jpeg",
      schema_version: "1",
    },
    displayName: options.displayName,
    metadata: options.metadata,
  });
}
