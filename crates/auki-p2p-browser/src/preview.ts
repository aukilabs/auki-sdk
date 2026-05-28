import type { OfferSummary, SpatialMessage } from "./peer.js";
import type { ByteSourceInput, PublicationHandle, PublishOfferOptions } from "./publication.js";

export const PREVIEW_OFFER_KIND = "auki.sensor.rgb_camera.preview";
export const PREVIEW_PAYLOAD_TYPE = "auki.camera.jpeg_frame.v1";
export const PREVIEW_PAYLOAD_ENCODING = "binary";
export const PREVIEW_PAYLOAD_MEDIA_TYPE = "image/jpeg";
export const PREVIEW_PAYLOAD_SCHEMA_VERSION = "1";
export const PREVIEW_ACCESS_MODES = ["get", "subscribe"] as const;

export type PreviewOfferOptions = {
  domainId: string;
  offerId: string;
  kind?: string;
  payloadType?: string;
  displayName?: string;
  metadata?: Record<string, unknown>;
  accessModes?: string[];
};

export type OfferPublisher = {
  publishOffer(options: PublishOfferOptions): Promise<PublicationHandle>;
};

export type PreviewFrame = {
  message: SpatialMessage;
  bytes: Uint8Array;
  sequence?: string;
  generatedAt?: string;
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
    accessModes: options.accessModes ?? [...PREVIEW_ACCESS_MODES],
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

export function isPreviewOffer(offer: OfferSummary): boolean {
  return offer.kind === PREVIEW_OFFER_KIND || offer.payloadType === PREVIEW_PAYLOAD_TYPE;
}

export function findPreviewOffer(
  offers: readonly OfferSummary[],
  selectOffer: (offer: OfferSummary) => boolean = isPreviewOffer,
): OfferSummary | undefined {
  return offers.find(selectOffer);
}

export function previewFrameFromMessage(message: SpatialMessage): PreviewFrame {
  const payload = jsonObjectField(message, "payload");
  const payloadType = stringField(payload, "type");
  if (payloadType !== PREVIEW_PAYLOAD_TYPE) {
    throw new Error(`Unsupported preview payload type ${payloadType}`);
  }
  return {
    message,
    bytes: decodeBase64UrlBytes(stringField(payload, "bytes")),
    sequence: optionalStringField(message, "sequence"),
    generatedAt: optionalStringField(message, "generated_at"),
  };
}

export function previewFrameBytes(message: SpatialMessage): Uint8Array {
  return previewFrameFromMessage(message).bytes;
}

export function decodeBase64UrlBytes(value: string): Uint8Array {
  const base64 = value.replaceAll("-", "+").replaceAll("_", "/");
  const padded = base64.padEnd(Math.ceil(base64.length / 4) * 4, "=");
  const binary = atob(padded);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index);
  }
  return bytes;
}

function jsonObjectField(value: SpatialMessage, field: string): Record<string, unknown> {
  const fieldValue = value[field];
  if (!fieldValue || typeof fieldValue !== "object" || Array.isArray(fieldValue)) {
    throw new Error(`Message missing object field ${field}`);
  }
  return fieldValue as Record<string, unknown>;
}

function stringField(value: Record<string, unknown>, field: string): string {
  const fieldValue = value[field];
  if (typeof fieldValue !== "string" || fieldValue.length === 0) {
    throw new Error(`Object missing string field ${field}`);
  }
  return fieldValue;
}

function optionalStringField(value: Record<string, unknown>, field: string): string | undefined {
  const fieldValue = value[field];
  return typeof fieldValue === "string" && fieldValue.length > 0 ? fieldValue : undefined;
}
