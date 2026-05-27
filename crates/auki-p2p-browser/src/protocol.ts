import initProtocolWasm, {
  createOfferCatalogRequest as wasmCreateOfferCatalogRequest,
  createPeerBinding as wasmCreatePeerBinding,
  createPeerHandshake as wasmCreatePeerHandshake,
  createSubscribeRequest as wasmCreateSubscribeRequest,
  decodeJsonFrame as wasmDecodeJsonFrame,
  decodeLength as wasmDecodeLength,
  encodeJsonFrame as wasmEncodeJsonFrame,
  encodeLength as wasmEncodeLength,
  parseOfferCatalogRequest as wasmParseOfferCatalogRequest,
  parseOfferCatalogResponse as wasmParseOfferCatalogResponse,
  parsePeerBinding as wasmParsePeerBinding,
  parsePeerHandshake as wasmParsePeerHandshake,
  parseSpatialMessage as wasmParseSpatialMessage,
  parseSubscribeEnd as wasmParseSubscribeEnd,
  parseSubscribeRequest as wasmParseSubscribeRequest,
  parseSubscribeStartResult as wasmParseSubscribeStartResult,
  protocolConstants as wasmProtocolConstants,
  protocolVersion as wasmProtocolVersion,
  validatePeerHandshakeAuthority as wasmValidatePeerHandshakeAuthority,
  validateSpatialMessageForOffer as wasmValidateSpatialMessageForOffer,
  validateSubscribeDataMessage as wasmValidateSubscribeDataMessage,
  validateSubscribeEndForOffer as wasmValidateSubscribeEndForOffer,
  validateSubscribeStartForRequest as wasmValidateSubscribeStartForRequest,
  verifyPeerBinding as wasmVerifyPeerBinding,
} from "../../auki-protocol-wasm/pkg-web/auki_protocol_wasm.js";

export type JsonObject = Record<string, unknown>;

export type DecodedLength = {
  value: number;
  consumed: number;
};

export type DecodedJsonFrame = {
  value: JsonObject;
  consumed: number;
};

type ProtocolWasmModuleInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export type ProtocolWasmInitInput =
  | ProtocolWasmModuleInput
  | Promise<ProtocolWasmModuleInput>
  | { module_or_path: ProtocolWasmModuleInput | Promise<ProtocolWasmModuleInput> };

export class ProtocolWasmError extends Error {
  readonly kind?: string;
  readonly failureCode?: string;
  readonly details: unknown;

  constructor(error: unknown) {
    const object = isObject(error) ? error : {};
    const message =
      typeof object.message === "string"
        ? object.message
        : error instanceof Error
          ? error.message
          : String(error);
    super(message);
    this.name = "ProtocolWasmError";
    this.kind = typeof object.kind === "string" ? object.kind : undefined;
    this.failureCode =
      typeof object.failure_code === "string" ? object.failure_code : undefined;
    this.details = error;
  }
}

let initPromise: Promise<void> | undefined;

export async function initializeProtocolWasm(input?: ProtocolWasmInitInput): Promise<void> {
  if (!initPromise) {
    initPromise = initProtocolWasm(input).then(
      () => undefined,
      (error) => {
        initPromise = undefined;
        throw normalizeProtocolError(error);
      },
    );
  }
  await initPromise;
}

export async function protocolVersion(): Promise<string> {
  return withProtocol(() => wasmProtocolVersion());
}

export async function protocolConstants(): Promise<JsonObject> {
  return withProtocol(() => expectJsonObject(wasmProtocolConstants(), "protocol constants"));
}

export async function encodeLength(value: number): Promise<Uint8Array> {
  return withProtocol(() => wasmEncodeLength(value));
}

export async function decodeLength(input: Uint8Array, maxBodyLen: number): Promise<DecodedLength> {
  return withProtocol(() => expectDecodedLength(wasmDecodeLength(input, maxBodyLen)));
}

export async function encodeJsonFrame(value: JsonObject, maxBodyLen: number): Promise<Uint8Array> {
  return withProtocol(() => wasmEncodeJsonFrame(value, maxBodyLen));
}

export async function decodeJsonFrame(
  input: Uint8Array,
  maxBodyLen: number,
): Promise<DecodedJsonFrame> {
  return withProtocol(() => expectDecodedJsonFrame(wasmDecodeJsonFrame(input, maxBodyLen)));
}

export async function createPeerBinding(
  walletSeed: Uint8Array,
  peerId: string,
  issuedAt: string,
  label?: string | null,
): Promise<JsonObject> {
  return withProtocol(() =>
    expectJsonObject(
      wasmCreatePeerBinding(walletSeed, peerId, issuedAt, label ?? undefined),
      "peer binding",
    ),
  );
}

export async function parsePeerBinding(value: JsonObject): Promise<JsonObject> {
  return withProtocol(() => expectJsonObject(wasmParsePeerBinding(value), "peer binding"));
}

export async function verifyPeerBinding(
  value: JsonObject,
  authenticatedPeerId: string,
): Promise<JsonObject> {
  return withProtocol(() =>
    expectJsonObject(wasmVerifyPeerBinding(value, authenticatedPeerId), "verified peer binding"),
  );
}

export async function createPeerHandshake(
  peerBinding: JsonObject,
  declaredDomains: JsonObject[] = [],
  offerCatalog?: JsonObject | null,
): Promise<JsonObject> {
  return withProtocol(() =>
    expectJsonObject(
      wasmCreatePeerHandshake(peerBinding, declaredDomains, offerCatalog ?? undefined),
      "peer handshake",
    ),
  );
}

export async function parsePeerHandshake(value: JsonObject): Promise<JsonObject> {
  return withProtocol(() => expectJsonObject(wasmParsePeerHandshake(value), "peer handshake"));
}

export async function validatePeerHandshakeAuthority(
  value: JsonObject,
  authenticatedPeerId: string,
  peerAuthorized: boolean,
  now: string,
): Promise<JsonObject> {
  return withProtocol(() =>
    expectJsonObject(
      wasmValidatePeerHandshakeAuthority(value, authenticatedPeerId, peerAuthorized, now),
      "peer handshake authority",
    ),
  );
}

export async function createOfferCatalogRequest(
  domainIds: string[] = [],
  kinds: string[] = [],
  includeInlineRegistryEntries = false,
): Promise<JsonObject> {
  return withProtocol(() =>
    expectJsonObject(
      wasmCreateOfferCatalogRequest(domainIds, kinds, includeInlineRegistryEntries),
      "offer catalog request",
    ),
  );
}

export async function parseOfferCatalogRequest(value: JsonObject): Promise<JsonObject> {
  return withProtocol(() =>
    expectJsonObject(wasmParseOfferCatalogRequest(value), "offer catalog request"),
  );
}

export async function parseOfferCatalogResponse(value: JsonObject): Promise<JsonObject> {
  return withProtocol(() =>
    expectJsonObject(wasmParseOfferCatalogResponse(value), "offer catalog response"),
  );
}

export async function createSubscribeRequest(
  domainId: string,
  offerId: string,
  params?: JsonObject | null,
  acceptedPayloadTypes: string[] = [],
  maxMessageBytes?: number | null,
): Promise<JsonObject> {
  return withProtocol(() =>
    expectJsonObject(
      wasmCreateSubscribeRequest(
        domainId,
        offerId,
        params ?? undefined,
        acceptedPayloadTypes,
        maxMessageBytes ?? undefined,
      ),
      "Subscribe request",
    ),
  );
}

export async function parseSubscribeRequest(value: JsonObject): Promise<JsonObject> {
  return withProtocol(() => expectJsonObject(wasmParseSubscribeRequest(value), "Subscribe request"));
}

export async function parseSubscribeStartResult(value: JsonObject): Promise<JsonObject> {
  return withProtocol(() =>
    expectJsonObject(wasmParseSubscribeStartResult(value), "Subscribe start result"),
  );
}

export async function validateSubscribeStartForRequest(
  request: JsonObject,
  startResult: JsonObject,
): Promise<JsonObject> {
  return withProtocol(() =>
    expectJsonObject(
      wasmValidateSubscribeStartForRequest(request, startResult),
      "Subscribe start validation",
    ),
  );
}

export async function parseSubscribeEnd(value: JsonObject): Promise<JsonObject> {
  return withProtocol(() => expectJsonObject(wasmParseSubscribeEnd(value), "Subscribe end"));
}

export async function validateSubscribeEndForOffer(
  end: JsonObject,
  domainId: string,
  offerId: string,
): Promise<JsonObject> {
  return withProtocol(() =>
    expectJsonObject(wasmValidateSubscribeEndForOffer(end, domainId, offerId), "Subscribe end"),
  );
}

export async function parseSpatialMessage(value: JsonObject): Promise<JsonObject> {
  return withProtocol(() => expectJsonObject(wasmParseSpatialMessage(value), "spatial message"));
}

export async function validateSpatialMessageForOffer(
  message: JsonObject,
  domainId: string,
  offerId: string,
  selectedPayloadType: string,
): Promise<JsonObject> {
  return withProtocol(() =>
    expectJsonObject(
      wasmValidateSpatialMessageForOffer(message, domainId, offerId, selectedPayloadType),
      "spatial message",
    ),
  );
}

export async function validateSubscribeDataMessage(
  acceptedStartResult: JsonObject,
  message: JsonObject,
  actualBodyLen?: number | null,
  maxMessageBytes?: number | null,
): Promise<JsonObject> {
  return withProtocol(() =>
    expectJsonObject(
      wasmValidateSubscribeDataMessage(
        acceptedStartResult,
        message,
        actualBodyLen ?? undefined,
        maxMessageBytes ?? undefined,
      ),
      "Subscribe data message",
    ),
  );
}

async function withProtocol<T>(operation: () => T): Promise<T> {
  await initializeProtocolWasm();
  try {
    return operation();
  } catch (error) {
    throw normalizeProtocolError(error);
  }
}

function normalizeProtocolError(error: unknown): ProtocolWasmError {
  return error instanceof ProtocolWasmError ? error : new ProtocolWasmError(error);
}

function expectDecodedLength(value: unknown): DecodedLength {
  const object = expectJsonObject(value, "decoded length");
  if (typeof object.value !== "number" || typeof object.consumed !== "number") {
    throw new Error("decoded length must include numeric value and consumed fields");
  }
  return { value: object.value, consumed: object.consumed };
}

function expectDecodedJsonFrame(value: unknown): DecodedJsonFrame {
  const object = expectJsonObject(value, "decoded JSON frame");
  if (typeof object.consumed !== "number") {
    throw new Error("decoded JSON frame must include numeric consumed field");
  }
  return {
    value: expectJsonObject(object.value, "decoded JSON frame value"),
    consumed: object.consumed,
  };
}

function expectJsonObject(value: unknown, label: string): JsonObject {
  if (!isObject(value) || Array.isArray(value)) {
    throw new Error(`${label} must be a JSON object`);
  }
  return value;
}

function isObject(value: unknown): value is JsonObject {
  return value !== null && typeof value === "object";
}
