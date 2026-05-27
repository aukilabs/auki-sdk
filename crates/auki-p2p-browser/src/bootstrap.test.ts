import { describe, expect, it } from "vitest";
import {
  bootstrapAddressBook,
  parseBootstrapRecord,
  preferredDialAddresses,
  relayServerAddresses,
} from "./bootstrap.js";

describe("browser bootstrap records", () => {
  it("parses Rust-shaped bootstrap records and preserves address roles", () => {
    const record = parseBootstrapRecord({
      peer_id: "12D3KooWNative",
      agent_version: "auki-p2p/0.0.0",
      direct_addresses: ["/ip4/127.0.0.1/udp/1/webrtc-direct/p2p/12D3KooWNative"],
      webrtc_direct_addresses: ["/ip4/127.0.0.1/udp/1/webrtc-direct/p2p/12D3KooWNative"],
      relay_addresses: [
        "/ip4/127.0.0.1/tcp/2/ws/p2p/12D3KooWRelay/p2p-circuit/p2p/12D3KooWNative",
      ],
      relay_server_addresses: ["/ip4/127.0.0.1/tcp/2/ws/p2p/12D3KooWNative"],
      bootstrap_addresses: [
        "/ip4/127.0.0.1/udp/1/webrtc-direct/p2p/12D3KooWNative",
        "/ip4/127.0.0.1/tcp/2/ws/p2p/12D3KooWNative",
      ],
    });

    expect(record.peerId).toBe("12D3KooWNative");
    expect(record.agentVersion).toBe("auki-p2p/0.0.0");
    expect(relayServerAddresses(record)).toEqual(["/ip4/127.0.0.1/tcp/2/ws/p2p/12D3KooWNative"]);
    expect(preferredDialAddresses(record)).toEqual([
      "/ip4/127.0.0.1/udp/1/webrtc-direct/p2p/12D3KooWNative",
      "/ip4/127.0.0.1/tcp/2/ws/p2p/12D3KooWNative",
      "/ip4/127.0.0.1/tcp/2/ws/p2p/12D3KooWRelay/p2p-circuit/p2p/12D3KooWNative",
    ]);
  });

  it("derives bootstrap addresses when older records only expose role arrays", () => {
    const record = parseBootstrapRecord({
      peer_id: "peer-a",
      direct_addresses: ["/memory/direct"],
      webrtc_direct_addresses: [],
      relay_addresses: ["/memory/relay"],
      relay_server_addresses: [],
    });

    expect(record.bootstrapAddresses).toEqual(["/memory/direct", "/memory/relay"]);
    expect(bootstrapAddressBook(record)).toEqual([
      { address: "/memory/direct", roles: ["bootstrap", "direct"] },
      { address: "/memory/relay", roles: ["bootstrap", "relay"] },
    ]);
  });

  it("rejects malformed records before they reach libp2p", () => {
    expect(() => parseBootstrapRecord({ peer_id: "", direct_addresses: [] })).toThrow(
      "missing peer_id",
    );
    expect(() =>
      parseBootstrapRecord({
        peer_id: "peer-a",
        direct_addresses: [1],
        webrtc_direct_addresses: [],
        relay_addresses: [],
        relay_server_addresses: [],
      }),
    ).toThrow("must contain only strings");
  });
});
