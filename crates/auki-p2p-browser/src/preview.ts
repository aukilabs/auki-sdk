import {
  parseOfferCatalogResponse,
  parseSubscribeStartResult,
  validateSubscribeEndForOffer,
  type JsonObject,
} from "./protocol.js";

export type OfferSummary = {
  peerId: string;
  domainId: string;
  offerId: string;
  kind?: string;
  payloadType?: string;
  accessModes: string[];
};

export type PreviewSource = AsyncIterable<Uint8Array> | Iterable<Uint8Array>;

export type PreviewOfferOptions = {
  domainId: string;
  offerId: string;
  kind?: string;
  payloadType?: string;
  displayName?: string;
  metadata?: JsonObject;
};

export type LoadedOffer = OfferSummary & {
  raw: JsonObject;
};

export type LocalPreviewPublication = {
  source: PreviewSource;
  offer: LoadedOffer;
  stopped: boolean;
  nextSequence: bigint;
};

export const DEFAULT_PREVIEW_OFFER_KIND = "auki.sensor.rgb_camera.preview";
export const DEFAULT_PREVIEW_PAYLOAD_TYPE = "auki.camera.jpeg_frame.v1";

export async function createPreviewOffer(
  peerId: string,
  options: PreviewOfferOptions,
): Promise<LoadedOffer> {
  const payloadType = options.payloadType ?? DEFAULT_PREVIEW_PAYLOAD_TYPE;
  const offer: JsonObject = {
    offer_id: options.offerId,
    domain_id: options.domainId,
    kind: options.kind ?? DEFAULT_PREVIEW_OFFER_KIND,
    status: "available",
    access_modes: ["subscribe"],
    payload: {
      type: payloadType,
      encoding: "binary",
      media_type: "image/jpeg",
      schema_version: "1",
    },
    registry_refs: [],
    updated_at: new Date().toISOString(),
  };
  if (options.displayName) {
    offer.display_name = options.displayName;
  }
  if (options.metadata) {
    offer.metadata = options.metadata;
  }

  const response = await parseOfferCatalogResponse({
    type: "auki.offer_catalog_response.v1",
    offers: [offer],
  });
  const offers = Array.isArray(response.offers) ? response.offers : [];
  return offerSummary(peerId, offers[0]);
}

export async function createLocalOfferCatalogResponse(
  publications: Iterable<LocalPreviewPublication>,
  request: JsonObject,
): Promise<JsonObject> {
  const domainIds = optionalStringArrayField(request, "domain_ids");
  const kinds = optionalStringArrayField(request, "kinds");
  const offers = Array.from(publications)
    .filter((publication) => !publication.stopped)
    .map((publication) => publication.offer)
    .filter((offer) => domainIds.length === 0 || domainIds.includes(offer.domainId))
    .filter((offer) => !offer.kind || kinds.length === 0 || kinds.includes(offer.kind))
    .map((offer) => offer.raw);

  return parseOfferCatalogResponse({
    type: "auki.offer_catalog_response.v1",
    offers,
    generated_at: new Date().toISOString(),
  });
}

export function offerSummary(peerId: string, value: unknown): LoadedOffer {
  if (!isJsonObject(value)) {
    throw new Error("Offer catalog response contains a non-object offer");
  }
  const payload = isJsonObject(value.payload) ? value.payload : undefined;
  return {
    peerId,
    domainId: stringField(value, "domain_id"),
    offerId: stringField(value, "offer_id"),
    kind: optionalStringField(value, "kind"),
    payloadType: payload ? optionalStringField(payload, "type") : undefined,
    accessModes: stringArrayField(value, "access_modes"),
    raw: value,
  };
}

export function offerKey(domainId: string, offerId: string): string {
  return `${domainId}\u0000${offerId}`;
}

export async function createSubscribeReject(
  request: JsonObject,
  code: string,
): Promise<JsonObject> {
  return parseSubscribeStartResult({
    type: "auki.subscribe_reject.v1",
    error: {
      code,
      domain_id: stringField(request, "domain_id"),
      offer_id: stringField(request, "offer_id"),
      retryable: code === "transport.failed" || code === "offer.temporarily_unavailable",
    },
  });
}

export async function createSubscribeAccept(offer: LoadedOffer): Promise<JsonObject> {
  return parseSubscribeStartResult({
    type: "auki.subscribe_accept.v1",
    domain_id: offer.domainId,
    offer_id: offer.offerId,
    payload: payloadDescriptor(offer.raw),
    generated_at: new Date().toISOString(),
  });
}

export async function createSubscribeEnd(
  offer: LoadedOffer,
  reason: "complete" | "offer_withdrawn",
): Promise<JsonObject> {
  const end: JsonObject = {
    type: "auki.subscribe_end.v1",
    domain_id: offer.domainId,
    offer_id: offer.offerId,
    reason,
  };
  if (reason === "offer_withdrawn") {
    end.retryable = true;
  }
  return validateSubscribeEndForOffer(end, offer.domainId, offer.offerId);
}

export async function createPreviewSpatialMessage(
  publication: LocalPreviewPublication,
  chunk: Uint8Array,
): Promise<JsonObject> {
  const sequence = publication.nextSequence;
  publication.nextSequence += 1n;
  const payload = {
    ...payloadDescriptor(publication.offer.raw),
    bytes: base64UrlEncode(chunk),
  };
  return {
    type: "auki.spatial_message.v1",
    domain_id: publication.offer.domainId,
    offer_id: publication.offer.offerId,
    payload,
    sequence: sequence.toString(),
    generated_at: new Date().toISOString(),
  };
}

export function requestAcceptsPayload(
  request: JsonObject,
  payloadType: string | undefined,
): boolean {
  if (!payloadType) {
    return false;
  }
  const acceptedPayloadTypes = optionalStringArrayField(request, "accepted_payload_types");
  return acceptedPayloadTypes.length === 0 || acceptedPayloadTypes.includes(payloadType);
}

export async function* toAsyncIterable(source: PreviewSource): AsyncIterable<Uint8Array> {
  if (isAsyncIterable(source)) {
    for await (const chunk of source) {
      yield chunk;
    }
    return;
  }
  for (const chunk of source) {
    yield chunk;
  }
}

export function stringField(value: JsonObject, field: string): string {
  const fieldValue = value[field];
  if (typeof fieldValue !== "string") {
    throw new Error(`JSON object missing string field ${field}`);
  }
  return fieldValue;
}

export function optionalNumberField(value: JsonObject, field: string): number | undefined {
  const fieldValue = value[field];
  if (fieldValue === undefined) {
    return undefined;
  }
  if (typeof fieldValue !== "number") {
    throw new Error(`Field ${field} must be a number`);
  }
  return fieldValue;
}

function payloadDescriptor(value: JsonObject): JsonObject {
  const payload = value.payload;
  if (!isJsonObject(payload)) {
    throw new Error("Offer missing payload descriptor");
  }
  return { ...payload };
}

function optionalStringField(value: JsonObject, field: string): string | undefined {
  const fieldValue = value[field];
  if (fieldValue === undefined) {
    return undefined;
  }
  if (typeof fieldValue !== "string") {
    throw new Error(`JSON object field ${field} must be a string`);
  }
  return fieldValue;
}

function stringArrayField(value: JsonObject, field: string): string[] {
  const fieldValue = value[field];
  if (!Array.isArray(fieldValue) || fieldValue.some((item) => typeof item !== "string")) {
    throw new Error(`JSON object field ${field} must be a string array`);
  }
  return fieldValue.slice();
}

function optionalStringArrayField(value: JsonObject, field: string): string[] {
  const fieldValue = value[field];
  if (fieldValue === undefined) {
    return [];
  }
  if (!Array.isArray(fieldValue) || fieldValue.some((item) => typeof item !== "string")) {
    throw new Error(`Field ${field} must be a string array`);
  }
  return fieldValue.slice();
}

function isAsyncIterable(source: PreviewSource): source is AsyncIterable<Uint8Array> {
  return Symbol.asyncIterator in source;
}

function base64UrlEncode(bytes: Uint8Array): string {
  const alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
  let output = "";
  for (let index = 0; index < bytes.byteLength; index += 3) {
    const first = bytes[index];
    const second = bytes[index + 1];
    const third = bytes[index + 2];
    output += alphabet[first >> 2];
    output += alphabet[((first & 0x03) << 4) | ((second ?? 0) >> 4)];
    if (second !== undefined) {
      output += alphabet[((second & 0x0f) << 2) | ((third ?? 0) >> 6)];
    }
    if (third !== undefined) {
      output += alphabet[third & 0x3f];
    }
  }
  return output;
}

function isJsonObject(value: unknown): value is JsonObject {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}
