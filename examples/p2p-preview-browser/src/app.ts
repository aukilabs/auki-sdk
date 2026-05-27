import {
  PREVIEW_OFFER_KIND,
  PREVIEW_PAYLOAD_TYPE,
  parseBootstrapRecord,
  type AukiBrowserBootstrapRecord,
  type OfferSummary,
  type SpatialMessage,
} from "@aukilabs/auki-p2p-browser";

export function parseBootstrapText(text: string): AukiBrowserBootstrapRecord {
  return parseBootstrapRecord(JSON.parse(text));
}

export function findPreviewOffer(offers: OfferSummary[]): OfferSummary | undefined {
  return offers.find(
    (offer) => offer.kind === PREVIEW_OFFER_KIND || offer.payloadType === PREVIEW_PAYLOAD_TYPE,
  );
}

export function previewFrameBytes(message: SpatialMessage): Uint8Array {
  const payload = jsonObjectField(message, "payload");
  const payloadType = stringField(payload, "type");
  if (payloadType !== PREVIEW_PAYLOAD_TYPE) {
    throw new Error(`Unsupported preview payload type ${payloadType}`);
  }
  return decodeBase64Url(stringField(payload, "bytes"));
}

export function offerLabel(offer: OfferSummary | undefined): string {
  if (!offer) {
    return "None";
  }
  return `${shortId(offer.domainId)}/${offer.offerId}`;
}

export function shortId(value: string, visible = 8): string {
  if (value.length <= visible * 2 + 1) {
    return value;
  }
  return `${value.slice(0, visible)}...${value.slice(-visible)}`;
}

export function decodeBase64Url(value: string): Uint8Array {
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
