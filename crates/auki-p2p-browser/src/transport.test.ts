import { describe, expect, it } from "vitest";
import { echoPingStream, type BrowserProtocolStream } from "./transport.js";

describe("browser transport lifecycle", () => {
  it("echoes standard libp2p ping payloads across chunk boundaries", async () => {
    const first = new Uint8Array(32).fill(1);
    const second = new Uint8Array(32).fill(2);
    const stream = new InputStream([
      first.slice(0, 7),
      concatBytes(first.slice(7), second.slice(0, 11)),
      second.slice(11),
    ]);

    await echoPingStream(stream);

    expect(stream.sent).toEqual([first, second]);
    expect(stream.closed).toBe(true);
  });

  it("does not echo incomplete ping payloads", async () => {
    const stream = new InputStream([new Uint8Array(31).fill(1)]);

    await echoPingStream(stream);

    expect(stream.sent).toEqual([]);
    expect(stream.closed).toBe(true);
  });
});

class InputStream implements BrowserProtocolStream {
  readonly sent: Uint8Array[] = [];
  closed = false;

  constructor(private readonly chunks: Uint8Array[]) {}

  send(data: Uint8Array): boolean {
    this.sent.push(data.slice());
    return true;
  }

  async close(): Promise<void> {
    this.closed = true;
  }

  async onDrain(): Promise<void> {}

  async *[Symbol.asyncIterator](): AsyncIterator<Uint8Array> {
    for (const chunk of this.chunks) {
      yield chunk;
    }
  }
}

function concatBytes(...parts: Uint8Array[]): Uint8Array {
  const output = new Uint8Array(parts.reduce((sum, part) => sum + part.byteLength, 0));
  let offset = 0;
  for (const part of parts) {
    output.set(part, offset);
    offset += part.byteLength;
  }
  return output;
}
