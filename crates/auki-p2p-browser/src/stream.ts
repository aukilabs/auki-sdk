import { decodeJsonFrame, decodeLength, encodeJsonFrame, type JsonObject } from "./protocol.js";
import type { BrowserProtocolStream } from "./transport.js";

export type ReadJsonFrame = {
  value: JsonObject;
  bodyLength: number;
  consumed: number;
};

export type StreamReadOptions = {
  signal?: AbortSignal;
};

const MAX_LEB128_U64_BYTES = 10;

export async function writeJsonFrame(
  stream: BrowserProtocolStream,
  value: JsonObject,
  maxBodyLen: number,
  options: StreamReadOptions = {},
): Promise<void> {
  throwIfAborted(options.signal);
  const frame = await encodeJsonFrame(value, maxBodyLen);
  if (!stream.send(frame) && stream.onDrain) {
    await abortable(stream.onDrain(), options.signal);
  }
}

export class JsonFrameReader {
  private readonly iterator: AsyncIterator<Uint8Array | { subarray(): Uint8Array }>;
  private buffer = new Uint8Array();

  constructor(stream: BrowserProtocolStream) {
    this.iterator = stream[Symbol.asyncIterator]();
  }

  async read(maxBodyLen: number, options: StreamReadOptions = {}): Promise<ReadJsonFrame> {
    throwIfAborted(options.signal);
    const length = await this.readLength(maxBodyLen, options);
    await this.ensure(length.consumed + length.value, options);

    const frame = this.buffer.slice(0, length.consumed + length.value);
    this.buffer = this.buffer.slice(frame.byteLength);
    const decoded = await decodeJsonFrame(frame, maxBodyLen);
    return {
      value: decoded.value,
      bodyLength: length.value,
      consumed: decoded.consumed,
    };
  }

  private async readLength(maxBodyLen: number, options: StreamReadOptions) {
    for (;;) {
      throwIfAborted(options.signal);
      if (this.buffer.byteLength >= MAX_LEB128_U64_BYTES) {
        return decodeLength(this.buffer, maxBodyLen);
      }

      try {
        return await decodeLength(this.buffer, maxBodyLen);
      } catch (error) {
        if (!isUnexpectedEof(error)) {
          throw error;
        }
        await this.readMore(options);
      }
    }
  }

  private async ensure(length: number, options: StreamReadOptions): Promise<void> {
    while (this.buffer.byteLength < length) {
      await this.readMore(options);
    }
  }

  private async readMore(options: StreamReadOptions): Promise<void> {
    const next = await abortable(this.iterator.next(), options.signal);
    if (next.done) {
      throw new Error("protocol stream closed before a complete frame arrived");
    }
    const chunk = normalizeChunk(next.value);
    const merged = new Uint8Array(this.buffer.byteLength + chunk.byteLength);
    merged.set(this.buffer);
    merged.set(chunk, this.buffer.byteLength);
    this.buffer = merged;
  }
}

function isUnexpectedEof(error: unknown): boolean {
  return error instanceof Error && error.message.includes("unexpected eof");
}

function normalizeChunk(chunk: Uint8Array | { subarray(): Uint8Array }): Uint8Array {
  return chunk instanceof Uint8Array ? chunk : chunk.subarray();
}

function abortable<T>(promise: Promise<T>, signal: AbortSignal | undefined): Promise<T> {
  if (!signal) {
    return promise;
  }
  if (signal.aborted) {
    promise.catch(() => undefined);
    return Promise.reject(abortReason(signal));
  }

  return new Promise<T>((resolve, reject) => {
    const abort = () => {
      cleanup();
      promise.catch(() => undefined);
      reject(abortReason(signal));
    };
    const cleanup = () => signal.removeEventListener("abort", abort);
    signal.addEventListener("abort", abort, { once: true });
    promise.then(
      (value) => {
        cleanup();
        resolve(value);
      },
      (error: unknown) => {
        cleanup();
        reject(error);
      },
    );
  });
}

function throwIfAborted(signal: AbortSignal | undefined): void {
  if (signal?.aborted) {
    throw abortReason(signal);
  }
}

function abortReason(signal: AbortSignal): Error {
  const reason = signal.reason;
  if (reason instanceof Error) {
    return reason;
  }
  if (typeof reason === "string") {
    return new Error(reason);
  }
  return new Error("stream aborted");
}
