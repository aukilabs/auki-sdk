import { readFile } from "node:fs/promises";
import path from "node:path";
import { describe, expect, it } from "vitest";
import { createAukiBrowserPeer } from "./peer.js";
import type { BrowserTransport } from "./transport.js";

describe("AukiBrowserPeer shell", () => {
  it("connects bootstrap records through an injected transport", async () => {
    const transport = new MemoryTransport("browser-peer", ["/p2p-circuit/p2p/browser-peer"]);
    const peer = await createAukiBrowserPeer({
      transport,
      bootstrap: bootstrapRecord("native-peer", "/memory/native-direct"),
      protocolWasm: await protocolWasmInput(),
    });

    expect(peer.peerId).toBe("browser-peer");
    expect(peer.supportedTransports).toContain("webrtc_direct");
    expect(peer.listPeers()).toEqual([
      {
        peerId: "native-peer",
        connected: false,
        dialAddresses: ["/memory/native-direct"],
      },
    ]);

    await peer.connectBootstrap([
      bootstrapRecord("native-peer", "/memory/native-direct"),
      bootstrapRecord("relay-peer", "/memory/relay"),
    ]);

    expect(transport.started).toBe(1);
    expect(transport.dials).toEqual([["/memory/native-direct"], ["/memory/relay"]]);
    expect(peer.listPeers()).toEqual([
      {
        peerId: "native-peer",
        connected: true,
        dialAddresses: ["/memory/native-direct"],
      },
      {
        peerId: "relay-peer",
        connected: true,
        dialAddresses: ["/memory/relay"],
      },
    ]);
  });

  it("stops the underlying transport", async () => {
    const transport = new MemoryTransport("browser-peer", []);
    const peer = await createAukiBrowserPeer({ transport, protocolWasm: await protocolWasmInput() });

    await peer.dial("/memory/native");
    await peer.stop();

    expect(transport.started).toBe(1);
    expect(transport.stopped).toBe(1);
  });
});

function bootstrapRecord(peerId: string, address: string): unknown {
  return {
    peer_id: peerId,
    direct_addresses: [address],
    webrtc_direct_addresses: [],
    relay_addresses: [],
    relay_server_addresses: [],
    bootstrap_addresses: [address],
  };
}

async function protocolWasmInput(): Promise<{ module_or_path: Uint8Array }> {
  return {
    module_or_path: await readFile(
      path.resolve(process.cwd(), "../auki-protocol-wasm/pkg-web/auki_protocol_wasm_bg.wasm"),
    ),
  };
}

class MemoryTransport implements BrowserTransport {
  readonly dials: string[][] = [];
  started = 0;
  stopped = 0;

  constructor(
    readonly peerId: string,
    private readonly addresses: string[],
  ) {}

  async start(): Promise<void> {
    this.started += 1;
  }

  async stop(): Promise<void> {
    this.stopped += 1;
  }

  multiaddrs(): string[] {
    return this.addresses.slice();
  }

  async dial(addresses: string[]): Promise<void> {
    this.dials.push(addresses.slice());
  }
}
