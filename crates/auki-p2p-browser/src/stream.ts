import { decodeJsonFrame, decodeLength, encodeJsonFrame, type JsonObject } from "./protocol.js";
import type { BrowserProtocolStream } from "./transport.js";

export type ReadJsonFrame = {
  value: JsonObject;
  bodyLength: number;
  consumed: number;
};

const MAX_LEB128_U64_BYTES = 10;

export async function writeJsonFrame(
  stream: BrowserProtocolStream,
  value: JsonObject,
  maxBodyLen: number,
): Promise<void> {
  const frame = await encodeJsonFrame(value, maxBodyLen);
  if (!stream.send(frame) && stream.onDrain) {
    await stream.onDrain();
  }
}

export class JsonFrameReader {
  private readonly iterator: AsyncIterator<Uint8Array | { subarray(): Uint8Array }>;
  private buffer = new Uint8Array();

  constructor(stream: BrowserProtocolStream) {
    this.iterator = stream[Symbol.asyncIterator]();
  }

  async read(maxBodyLen: number): Promise<ReadJsonFrame> {
    const length = await this.readLength(maxBodyLen);
    await this.ensure(length.consumed + length.value);

    const frame = this.buffer.slice(0, length.consumed + length.value);
    this.buffer = this.buffer.slice(frame.byteLength);
    const decoded = await decodeJsonFrame(frame, maxBodyLen);
    return {
      value: decoded.value,
      bodyLength: length.value,
      consumed: decoded.consumed,
    };
  }

  private async readLength(maxBodyLen: number) {
    for (;;) {
      if (this.buffer.byteLength >= MAX_LEB128_U64_BYTES) {
        return decodeLength(this.buffer, maxBodyLen);
      }

      try {
        return await decodeLength(this.buffer, maxBodyLen);
      } catch (error) {
        if (!isUnexpectedEof(error)) {
          throw error;
        }
        await this.readMore();
      }
    }
  }

  private async ensure(length: number): Promise<void> {
    while (this.buffer.byteLength < length) {
      await this.readMore();
    }
  }

  private async readMore(): Promise<void> {
    const next = await this.iterator.next();
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
