import { readFile } from "node:fs/promises";
import path from "node:path";
import { describe, expect, it } from "vitest";
import { initializeProtocolWasm } from "./protocol.js";
import { JsonFrameReader, writeJsonFrame } from "./stream.js";
import type { BrowserProtocolStream } from "./transport.js";

describe("JSON protocol streams", () => {
  it("aborts a pending frame read without waiting for the transport iterator", async () => {
    await initializeProtocolWasm(await protocolWasmInput());
    const abort = new AbortController();
    const reader = new JsonFrameReader(new PendingStream());
    const read = reader.read(1_024, { signal: abort.signal });

    abort.abort(new Error("Stopped by user"));

    await expect(read).rejects.toThrow("Stopped by user");
  });

  it("aborts a pending write drain", async () => {
    await initializeProtocolWasm(await protocolWasmInput());
    const abort = new AbortController();
    const stream = new BackpressuredStream();
    const write = writeJsonFrame(stream, { type: "example" }, 1_024, {
      signal: abort.signal,
    });

    abort.abort(new Error("Write stopped"));

    await expect(write).rejects.toThrow("Write stopped");
  });
});

class PendingStream implements BrowserProtocolStream {
  send(): boolean {
    return true;
  }

  async close(): Promise<void> {}

  async *[Symbol.asyncIterator](): AsyncIterator<Uint8Array> {
    await new Promise(() => undefined);
  }
}

class BackpressuredStream implements BrowserProtocolStream {
  send(): boolean {
    return false;
  }

  async close(): Promise<void> {}

  async onDrain(): Promise<void> {
    await new Promise(() => undefined);
  }

  async *[Symbol.asyncIterator](): AsyncIterator<Uint8Array> {}
}

async function protocolWasmInput(): Promise<{ module_or_path: Uint8Array }> {
  return {
    module_or_path: await readFile(
      path.resolve(process.cwd(), "../auki-protocol-wasm/pkg-web/auki_protocol_wasm_bg.wasm"),
    ),
  };
}
