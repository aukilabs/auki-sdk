import type { ByteSourceInput, PublicationHandle, PublishOfferOptions } from "./publication.js";

export const PREVIEW_OFFER_KIND = "auki.sensor.rgb_camera.preview";
export const PREVIEW_PAYLOAD_TYPE = "auki.camera.jpeg_frame.v1";
export const PREVIEW_PAYLOAD_ENCODING = "binary";
export const PREVIEW_PAYLOAD_MEDIA_TYPE = "image/jpeg";
export const PREVIEW_PAYLOAD_SCHEMA_VERSION = "1";

export type PreviewOfferOptions = {
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

export function previewPayloadDescriptor(
  payloadType = PREVIEW_PAYLOAD_TYPE,
): Record<string, unknown> {
  return {
    type: payloadType,
    encoding: PREVIEW_PAYLOAD_ENCODING,
    media_type: PREVIEW_PAYLOAD_MEDIA_TYPE,
    schema_version: PREVIEW_PAYLOAD_SCHEMA_VERSION,
  };
}

export function publishPreviewOffer(
  peer: OfferPublisher,
  source: ByteSourceInput,
  options: PreviewOfferOptions,
): Promise<PublicationHandle> {
  return peer.publishOffer({
    source,
    domainId: options.domainId,
    offerId: options.offerId,
    kind: options.kind ?? PREVIEW_OFFER_KIND,
    payload: previewPayloadDescriptor(options.payloadType),
    displayName: options.displayName,
    metadata: options.metadata,
  });
}

export const GENERATED_PREVIEW_OFFER_KIND = PREVIEW_OFFER_KIND;
export const GENERATED_PREVIEW_PAYLOAD_TYPE = PREVIEW_PAYLOAD_TYPE;
export type GeneratedPreviewOptions = PreviewOfferOptions;

export function publishGeneratedPreview(
  peer: OfferPublisher,
  source: ByteSourceInput,
  options: GeneratedPreviewOptions,
): Promise<PublicationHandle> {
  return publishPreviewOffer(peer, source, options);
}
