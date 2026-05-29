import { describe, expect, it } from "vitest";
import {
  browserPeerConnectionCleanupComplete,
  browserReachableMultiaddrs,
  echoPingStream,
  openBrowserProtocolStream,
  preferredBrowserConnectionAddresses,
  type BrowserProtocolStream,
} from "./transport.js";

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

  it("skips closing retained connections when opening protocol streams", async () => {
    const stale = new FakeConnection(
      "/memory/webrtc-direct",
      new Error('The connection muxer is "closing" and not "open"'),
    );
    const fresh = new FakeConnection("/memory/webrtc-direct");
    const dials: Array<{ addresses: string[]; force?: boolean }> = [];

    const stream = await openBrowserProtocolStream(
      [stale],
      ["/memory/webrtc-direct"],
      "/auki/get/0.0.1",
      async (addresses, options) => {
        dials.push({ addresses: addresses.slice(), force: options.force });
        return fresh;
      },
    );

    expect(stream).toBe(fresh.stream);
    expect(stale.closed).toBe(true);
    expect(dials).toEqual([
      { addresses: ["/memory/webrtc-direct"], force: false },
    ]);
  });

  it("opens protocol streams on existing connections without bootstrap addresses", async () => {
    const existing = new FakeConnection("/memory/relay/browser-peer");
    const dials: Array<{ addresses: string[]; force?: boolean }> = [];

    const stream = await openBrowserProtocolStream(
      [existing],
      [],
      "/auki/offer_catalog/0.0.1",
      async (addresses, options) => {
        dials.push({ addresses: addresses.slice(), force: options.force });
        throw new Error("dial should not be needed");
      },
    );

    expect(stream).toBe(existing.stream);
    expect(dials).toEqual([]);
  });

  it("rejects protocol streams without a live connection or bootstrap addresses", async () => {
    await expect(
      openBrowserProtocolStream([], [], "/auki/get/0.0.1", async () => {
        throw new Error("dial should not be needed");
      }),
    ).rejects.toThrow("No active connection or bootstrap addresses available");
  });

  it("force dials when a freshly dialed connection has a closing muxer", async () => {
    const closingDial = new FakeConnection(
      "/memory/websocket",
      new Error('The connection muxer is "closed" and not "open"'),
    );
    const forcedDial = new FakeConnection("/memory/websocket");
    const dials: Array<{ addresses: string[]; force?: boolean }> = [];

    const stream = await openBrowserProtocolStream(
      [],
      ["/memory/websocket"],
      "/auki/subscribe/0.0.1",
      async (addresses, options) => {
        dials.push({ addresses: addresses.slice(), force: options.force });
        return options.force ? forcedDial : closingDial;
      },
    );

    expect(stream).toBe(forcedDial.stream);
    expect(closingDial.closed).toBe(true);
    expect(dials).toEqual([
      { addresses: ["/memory/websocket"], force: false },
      { addresses: ["/memory/websocket"], force: true },
    ]);
  });

  it("does not complete selected-address cleanup while stale paths are still closing", () => {
    const selected = cleanupConnection("ws-1", "/memory/websocket", "open");
    const stale = cleanupConnection("rtc-1", "/memory/webrtc-direct", "closing");

    expect(
      browserPeerConnectionCleanupComplete(
        [selected, stale],
        ["/memory/websocket"],
      ),
    ).toBe(false);

    stale.status = "closed";

    expect(
      browserPeerConnectionCleanupComplete(
        [selected, stale],
        ["/memory/websocket"],
      ),
    ).toBe(true);
  });

  it("prefers direct WebRTC over a relayed websocket path", () => {
    expect(
      preferredBrowserConnectionAddresses([
        cleanupConnection(
          "relay",
          "/ip4/127.0.0.1/tcp/1/ws/p2p/relay/p2p-circuit/p2p/browser",
          "open",
          false,
          { seconds: 120 },
        ),
        cleanupConnection("webrtc", "/webrtc/p2p/browser", "open", true),
      ]),
    ).toEqual(["/webrtc/p2p/browser"]);
  });

  it("keeps the only open path even when it is relayed", () => {
    expect(
      preferredBrowserConnectionAddresses([
        cleanupConnection(
          "relay",
          "/ip4/127.0.0.1/tcp/1/ws/p2p/relay/p2p-circuit/p2p/browser",
          "open",
          false,
          { seconds: 120 },
        ),
      ]),
    ).toEqual(["/ip4/127.0.0.1/tcp/1/ws/p2p/relay/p2p-circuit/p2p/browser"]);
  });

  it("does not fabricate relay reservations from relay-server addresses", () => {
    expect(
      browserReachableMultiaddrs(
        "browser-peer",
        [
          "/ip4/127.0.0.1/tcp/1/ws/p2p/browser-peer",
          "/ip4/127.0.0.1/tcp/2/ws/p2p/relay-peer",
          "/ip4/127.0.0.1/tcp/2/ws/p2p/relay-peer/p2p-circuit/webrtc/p2p/browser-peer",
          "/ip4/127.0.0.1/tcp/2/ws/p2p/relay-peer/p2p-circuit/p2p/browser-peer",
          "/webrtc/p2p/browser-peer",
        ],
      ),
    ).toEqual([
      "/ip4/127.0.0.1/tcp/1/ws/p2p/browser-peer",
      "/ip4/127.0.0.1/tcp/2/ws/p2p/relay-peer/p2p-circuit/p2p/browser-peer",
    ]);
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

function cleanupConnection(
  id: string,
  address: string,
  status: string,
  direct = true,
  limits?: unknown,
) {
  return {
    id,
    remoteAddr: { toString: () => address },
    status,
    direct,
    limits,
  };
}

class FakeConnection {
  readonly id: string;
  readonly remoteAddr: { toString(): string };
  readonly status = "open";
  readonly stream = new InputStream([]);
  closed = false;

  constructor(
    address: string,
    private readonly newStreamError?: Error,
  ) {
    this.id = address;
    this.remoteAddr = { toString: () => address };
  }

  async newStream(): Promise<BrowserProtocolStream> {
    if (this.newStreamError) {
      throw this.newStreamError;
    }
    return this.stream;
  }

  async close(): Promise<void> {
    this.closed = true;
  }
}
