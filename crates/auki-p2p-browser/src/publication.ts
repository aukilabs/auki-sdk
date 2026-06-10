import {
  parseGetResponse,
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

export type PublishedByteFrame = {
  readonly bytes: Uint8Array;
  readonly sequence?: string | number | bigint;
  readonly generatedAt?: string;
};
export type PublishedByteFrameInput = Uint8Array | PublishedByteFrame;
export type ByteSource =
  | AsyncIterable<PublishedByteFrameInput>
  | Iterable<PublishedByteFrameInput>;
export type ByteSourceFactory = () => ByteSource;
export type ByteSourceInput = ByteSource | ByteSourceFactory;

export const DEFAULT_SUBSCRIPTION_QUEUE_CAPACITY = 1024;
export const SUBSCRIBE_BACKPRESSURE_ERROR_CODE = "subscribe.backpressure";

export type AukiSubscriptionBackpressurePolicy =
  | { readonly kind: "LatestOnly" }
  | { readonly kind: "Bounded"; readonly capacity?: number }
  | { readonly kind: "CloseOnFull"; readonly capacity?: number };

export type NormalizedSubscriptionBackpressurePolicy =
  | { readonly kind: "LatestOnly" }
  | { readonly kind: "Bounded"; readonly capacity: number }
  | { readonly kind: "CloseOnFull"; readonly capacity: number };

export const DEFAULT_SUBSCRIPTION_BACKPRESSURE_POLICY: NormalizedSubscriptionBackpressurePolicy = {
  kind: "Bounded",
  capacity: DEFAULT_SUBSCRIPTION_QUEUE_CAPACITY,
};

export const LATEST_ONLY_SUBSCRIPTION_BACKPRESSURE_POLICY: NormalizedSubscriptionBackpressurePolicy =
  {
    kind: "LatestOnly",
  };

export type PublishOfferOptions = {
  source: ByteSourceInput;
  domainId: string;
  offerId: string;
  kind: string;
  payload: JsonObject;
  displayName?: string;
  metadata?: JsonObject;
  registryRefs?: JsonObject[];
  accessModes?: string[];
  backpressurePolicy?: AukiSubscriptionBackpressurePolicy;
};

export type PublicationHandle = {
  readonly domainId: string;
  readonly offerId: string;
  stop(): Promise<void>;
};

export type LoadedOffer = OfferSummary & {
  raw: JsonObject;
};

export type LocalOfferPublication = {
  source: ByteSourceInput;
  offer: LoadedOffer;
  stopped: boolean;
  nextSequence: bigint;
  backpressurePolicy: NormalizedSubscriptionBackpressurePolicy;
};

export type SubscriptionSourceEvent =
  | { kind: "chunk"; chunk: PublishedByteFrameInput }
  | { kind: "complete" }
  | { kind: "close_for_backpressure" };

type LatestPublishedByteSourceRead =
  | {
      kind: "frame";
      frame: PublishedByteFrame;
      version: number;
    }
  | { kind: "pending" }
  | { kind: "closed" };

export class LatestPublishedByteSource implements AsyncIterable<PublishedByteFrame> {
  private latestFrame?: PublishedByteFrame;
  private frameVersion = 0;
  private closed = false;
  private readonly waiters = new Set<() => void>();

  publish(frame: PublishedByteFrameInput): boolean {
    if (this.closed) {
      return false;
    }
    this.latestFrame = clonePublishedByteFrame(normalizePublishedByteFrame(frame));
    this.frameVersion += 1;
    this.wake();
    return true;
  }

  latest(): PublishedByteFrame | undefined {
    return this.latestFrame ? clonePublishedByteFrame(this.latestFrame) : undefined;
  }

  latestBytes(): Uint8Array | undefined {
    return this.latestFrame?.bytes.slice();
  }

  close(): void {
    if (this.closed) {
      return;
    }
    this.closed = true;
    this.wake();
  }

  isClosed(): boolean {
    return this.closed;
  }

  stream(): AsyncIterable<PublishedByteFrame> {
    return this;
  }

  async *[Symbol.asyncIterator](): AsyncIterator<PublishedByteFrame> {
    let lastVersion = 0;
    for (;;) {
      const read = this.readAfter(lastVersion);
      if (read.kind === "frame") {
        lastVersion = read.version;
        yield read.frame;
        continue;
      }
      if (read.kind === "closed") {
        return;
      }
      await this.waitForFrame();
    }
  }

  private readAfter(lastVersion: number): LatestPublishedByteSourceRead {
    if (this.latestFrame && this.frameVersion > lastVersion) {
      return {
        kind: "frame",
        frame: clonePublishedByteFrame(this.latestFrame),
        version: this.frameVersion,
      };
    }
    return this.closed ? { kind: "closed" } : { kind: "pending" };
  }

  private waitForFrame(): Promise<void> {
    return new Promise((resolve) => {
      this.waiters.add(resolve);
    });
  }

  private wake(): void {
    for (const resolve of this.waiters) {
      resolve();
    }
    this.waiters.clear();
  }
}

export async function createPublishedOffer(
  peerId: string,
  options: PublishOfferOptions,
): Promise<LoadedOffer> {
  const offer: JsonObject = {
    offer_id: options.offerId,
    domain_id: options.domainId,
    kind: options.kind,
    status: "available",
    access_modes: options.accessModes ?? ["subscribe"],
    payload: { ...options.payload },
    registry_refs: options.registryRefs ?? [],
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
  publications: Iterable<LocalOfferPublication>,
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

export async function createGetFailure(request: JsonObject, code: string): Promise<JsonObject> {
  return parseGetResponse({
    type: "auki.get_response.v1",
    error: {
      code,
      domain_id: stringField(request, "domain_id"),
      offer_id: stringField(request, "offer_id"),
      retryable: code === "transport.failed" || code === "offer.temporarily_unavailable",
    },
  });
}

export async function createGetSuccess(message: JsonObject): Promise<JsonObject> {
  return parseGetResponse({
    type: "auki.get_response.v1",
    message,
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

export type SubscribeEndReason =
  | "complete"
  | "cancelled"
  | "offer_withdrawn"
  | "not_authorized"
  | "producer_shutdown"
  | "error";

export async function createSubscribeEnd(
  offer: LoadedOffer,
  reason: SubscribeEndReason,
  options: SubscribeEndOptions = {},
): Promise<JsonObject> {
  return createSubscribeEndForPath(offer.domainId, offer.offerId, reason, options);
}

export type SubscribeEndOptions = {
  errorCode?: string;
  retryable?: boolean;
  details?: JsonObject;
};

export async function createSubscribeEndForPath(
  domainId: string,
  offerId: string,
  reason: SubscribeEndReason,
  options: SubscribeEndOptions = {},
): Promise<JsonObject> {
  const end: JsonObject = {
    type: "auki.subscribe_end.v1",
    domain_id: domainId,
    offer_id: offerId,
    reason,
  };
  if (options.errorCode) {
    end.error = { code: options.errorCode };
  }
  if (options.retryable !== undefined) {
    end.retryable = options.retryable;
  } else if (reason === "offer_withdrawn") {
    end.retryable = true;
  }
  if (options.details) {
    end.details = options.details;
  }
  return validateSubscribeEndForOffer(end, domainId, offerId);
}

export async function createPublicationSpatialMessage(
  publication: LocalOfferPublication,
  chunk: PublishedByteFrameInput,
): Promise<JsonObject> {
  const frame = normalizePublishedByteFrame(chunk);
  const sequence = sequenceString(frame.sequence, publication.nextSequence);
  if (frame.sequence === undefined) {
    publication.nextSequence += 1n;
  } else {
    publication.nextSequence = maxBigint(publication.nextSequence, BigInt(sequence) + 1n);
  }
  const payload = {
    ...payloadDescriptor(publication.offer.raw),
    bytes: base64UrlEncode(frame.bytes),
  };
  return {
    type: "auki.spatial_message.v1",
    domain_id: publication.offer.domainId,
    offer_id: publication.offer.offerId,
    payload,
    sequence,
    generated_at: frame.generatedAt ?? new Date().toISOString(),
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

export async function* toAsyncIterable(
  source: ByteSourceInput,
): AsyncIterable<PublishedByteFrameInput> {
  const opened = openByteSource(source);
  if (isAsyncIterable(opened)) {
    for await (const chunk of opened) {
      yield chunk;
    }
    return;
  }
  for (const chunk of opened) {
    yield chunk;
  }
}

export function normalizeSubscriptionBackpressurePolicy(
  policy: AukiSubscriptionBackpressurePolicy | undefined,
): NormalizedSubscriptionBackpressurePolicy {
  if (!policy) {
    return DEFAULT_SUBSCRIPTION_BACKPRESSURE_POLICY;
  }
  if (policy.kind === "LatestOnly") {
    return LATEST_ONLY_SUBSCRIPTION_BACKPRESSURE_POLICY;
  }
  return {
    kind: policy.kind,
    capacity: normalizeBackpressureCapacity(policy.capacity),
  };
}

export function openBackpressuredByteSource(
  source: ByteSourceInput,
  policy: AukiSubscriptionBackpressurePolicy | NormalizedSubscriptionBackpressurePolicy,
): AsyncIterator<SubscriptionSourceEvent> {
  const normalized = normalizeSubscriptionBackpressurePolicy(policy);
  const sourceIterator = toAsyncIterable(source)[Symbol.asyncIterator]();
  const queue = new SubscriptionSourceQueue(normalized);
  const producer = pumpSubscriptionSource(sourceIterator, queue, normalized);

  return {
    async next(): Promise<IteratorResult<SubscriptionSourceEvent>> {
      const event = await queue.pop();
      if (!event) {
        return { done: true, value: undefined };
      }
      return { done: false, value: event };
    },
    async return(): Promise<IteratorResult<SubscriptionSourceEvent>> {
      queue.close();
      await sourceIterator.return?.();
      await producer.catch(() => undefined);
      return { done: true, value: undefined };
    },
  };
}

async function pumpSubscriptionSource(
  source: AsyncIterator<PublishedByteFrameInput>,
  queue: SubscriptionSourceQueue,
  policy: NormalizedSubscriptionBackpressurePolicy,
): Promise<void> {
  try {
    for (;;) {
      const next = await source.next();
      if (next.done) {
        await queue.push(policy, { kind: "complete" });
        return;
      }
      if (!(await queue.push(policy, { kind: "chunk", chunk: next.value }))) {
        await source.return?.();
        return;
      }
    }
  } catch (error) {
    queue.fail(error);
  }
}

class SubscriptionSourceQueue {
  private readonly queue: SubscriptionSourceEvent[] = [];
  private readonly itemWaiters = new Set<() => void>();
  private readonly spaceWaiters = new Set<() => void>();
  private closed = false;
  private failure: unknown;

  constructor(private readonly defaultPolicy: NormalizedSubscriptionBackpressurePolicy) {}

  async push(
    policy: NormalizedSubscriptionBackpressurePolicy = this.defaultPolicy,
    event: SubscriptionSourceEvent,
  ): Promise<boolean> {
    switch (policy.kind) {
      case "LatestOnly":
        return this.pushLatestOnly(event);
      case "Bounded":
        return this.pushBounded(policy.capacity, event);
      case "CloseOnFull":
        return this.pushCloseOnFull(policy.capacity, event);
    }
  }

  async pop(): Promise<SubscriptionSourceEvent | undefined> {
    for (;;) {
      const event = this.queue.shift();
      if (event) {
        this.wakeSpace();
        return event;
      }
      if (this.failure) {
        throw this.failure;
      }
      if (this.closed) {
        return undefined;
      }
      await this.waitForItem();
    }
  }

  close(): void {
    if (this.closed) {
      return;
    }
    this.closed = true;
    this.wakeItem();
    this.wakeSpace();
  }

  fail(error: unknown): void {
    this.failure = error;
    this.closed = true;
    this.wakeItem();
    this.wakeSpace();
  }

  private pushLatestOnly(event: SubscriptionSourceEvent): boolean {
    if (this.closed) {
      return false;
    }
    if (event.kind === "chunk") {
      this.queue.splice(0, this.queue.length, event);
      this.wakeItem();
      return true;
    }
    if (event.kind === "close_for_backpressure") {
      this.queue.splice(0, this.queue.length, event);
    } else {
      this.queue.push(event);
    }
    this.closed = true;
    this.wakeItem();
    this.wakeSpace();
    return true;
  }

  private async pushBounded(
    capacity: number,
    event: SubscriptionSourceEvent,
  ): Promise<boolean> {
    for (;;) {
      if (this.closed) {
        return false;
      }
      if (this.queue.length < capacity) {
        this.queue.push(event);
        if (event.kind !== "chunk") {
          this.closed = true;
          this.wakeSpace();
        }
        this.wakeItem();
        return true;
      }
      await this.waitForSpace();
    }
  }

  private pushCloseOnFull(capacity: number, event: SubscriptionSourceEvent): boolean {
    if (this.closed) {
      return false;
    }
    if (event.kind === "chunk" && this.queue.length >= capacity) {
      this.queue.splice(0, this.queue.length, { kind: "close_for_backpressure" });
      this.closed = true;
      this.wakeItem();
      this.wakeSpace();
      return false;
    }
    if (event.kind === "close_for_backpressure") {
      this.queue.splice(0, this.queue.length, event);
    } else {
      this.queue.push(event);
    }
    if (event.kind !== "chunk") {
      this.closed = true;
      this.wakeSpace();
    }
    this.wakeItem();
    return true;
  }

  private waitForItem(): Promise<void> {
    return new Promise((resolve) => this.itemWaiters.add(resolve));
  }

  private waitForSpace(): Promise<void> {
    return new Promise((resolve) => this.spaceWaiters.add(resolve));
  }

  private wakeItem(): void {
    for (const resolve of this.itemWaiters) {
      resolve();
    }
    this.itemWaiters.clear();
  }

  private wakeSpace(): void {
    for (const resolve of this.spaceWaiters) {
      resolve();
    }
    this.spaceWaiters.clear();
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

function openByteSource(source: ByteSourceInput): ByteSource {
  return typeof source === "function" ? source() : source;
}

function normalizeBackpressureCapacity(capacity: number | undefined): number {
  if (capacity === undefined) {
    return DEFAULT_SUBSCRIPTION_QUEUE_CAPACITY;
  }
  if (!Number.isSafeInteger(capacity) || capacity < 1) {
    throw new Error("Backpressure capacity must be a positive safe integer");
  }
  return capacity;
}

function isAsyncIterable(source: ByteSource): source is AsyncIterable<PublishedByteFrameInput> {
  return Symbol.asyncIterator in source;
}

function normalizePublishedByteFrame(frame: PublishedByteFrameInput): PublishedByteFrame {
  if (frame instanceof Uint8Array) {
    return { bytes: frame };
  }
  if (!(frame.bytes instanceof Uint8Array)) {
    throw new Error("Published byte frame bytes must be a Uint8Array");
  }
  return frame;
}

function clonePublishedByteFrame(frame: PublishedByteFrame): PublishedByteFrame {
  return {
    bytes: frame.bytes.slice(),
    ...(frame.sequence === undefined ? {} : { sequence: frame.sequence }),
    ...(frame.generatedAt === undefined ? {} : { generatedAt: frame.generatedAt }),
  };
}

function sequenceString(
  sequence: PublishedByteFrame["sequence"],
  fallback: bigint,
): string {
  if (sequence === undefined) {
    return fallback.toString();
  }
  if (typeof sequence === "bigint") {
    if (sequence < 0n) {
      throw new Error("Published byte frame sequence must be non-negative");
    }
    return sequence.toString();
  }
  if (typeof sequence === "number") {
    if (!Number.isSafeInteger(sequence) || sequence < 0) {
      throw new Error("Published byte frame sequence must be a non-negative safe integer");
    }
    return sequence.toString();
  }
  if (!/^(0|[1-9]\d*)$/.test(sequence)) {
    throw new Error("Published byte frame sequence must be a non-negative integer string");
  }
  return sequence;
}

function maxBigint(left: bigint, right: bigint): bigint {
  return left > right ? left : right;
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
