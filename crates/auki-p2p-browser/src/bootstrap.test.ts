import { describe, expect, it } from "vitest";
import {
  bootstrapAddressBook,
  createLocalBootstrapRecord,
  isExportableBrowserBootstrapAddress,
  parseBootstrapRecord,
  parseBootstrapRecords,
  preferredDialAddresses,
  relayServerAddresses,
} from "./bootstrap.js";

describe("browser bootstrap records", () => {
  it("creates local browser records from browser-target addresses", () => {
    const record = createLocalBootstrapRecord("browser-peer", [
      "/ip4/127.0.0.1/tcp/2/ws/p2p/relay-peer",
      "/ip4/127.0.0.1/udp/1/webrtc-direct/p2p/browser-peer",
      "/ip4/127.0.0.1/tcp/2/ws/p2p/relay-peer/p2p-circuit/p2p/browser-peer",
      "/ip4/127.0.0.1/tcp/2/ws/p2p/relay-peer/p2p-circuit/p2p/browser-peer",
    ]);

    expect(record).toEqual({
      peerId: "browser-peer",
      agentVersion: "auki-p2p-browser/0.0.0",
      directAddresses: ["/ip4/127.0.0.1/udp/1/webrtc-direct/p2p/browser-peer"],
      webrtcDirectAddresses: ["/ip4/127.0.0.1/udp/1/webrtc-direct/p2p/browser-peer"],
      relayAddresses: [
        "/ip4/127.0.0.1/tcp/2/ws/p2p/relay-peer/p2p-circuit/p2p/browser-peer",
      ],
      relayServerAddresses: [],
      bootstrapAddresses: [
        "/ip4/127.0.0.1/udp/1/webrtc-direct/p2p/browser-peer",
        "/ip4/127.0.0.1/tcp/2/ws/p2p/relay-peer/p2p-circuit/p2p/browser-peer",
      ],
    });
  });

  it("does not export relay servers as local browser peer addresses", () => {
    expect(() =>
      createLocalBootstrapRecord("browser-peer", [
        "/ip4/127.0.0.1/tcp/2/ws/p2p/relay-peer",
      ]),
    ).toThrow("not dialable yet");
  });

  it("does not export transient browser WebRTC observed paths as bootstrap addresses", () => {
    const stableRelay =
      "/ip4/127.0.0.1/tcp/2/ws/p2p/relay-peer/p2p-circuit/p2p/browser-peer";
    const transientRelayWebrtc =
      "/ip4/127.0.0.1/tcp/2/ws/p2p/relay-peer/p2p-circuit/webrtc/p2p/browser-peer";
    const transientDirectWebrtc = "/webrtc/p2p/browser-peer";

    const record = createLocalBootstrapRecord("browser-peer", [
      transientRelayWebrtc,
      transientDirectWebrtc,
      stableRelay,
    ]);

    expect(record.directAddresses).toEqual([]);
    expect(record.relayAddresses).toEqual([stableRelay]);
    expect(record.bootstrapAddresses).toEqual([stableRelay]);
  });

  it("classifies only durable browser bootstrap addresses as exportable", () => {
    expect(
      isExportableBrowserBootstrapAddress(
        "/ip4/127.0.0.1/tcp/1/ws/p2p/browser-peer",
        "browser-peer",
      ),
    ).toBe(true);
    expect(
      isExportableBrowserBootstrapAddress(
        "/ip4/127.0.0.1/udp/1/webrtc-direct/p2p/browser-peer",
        "browser-peer",
      ),
    ).toBe(true);
    expect(
      isExportableBrowserBootstrapAddress(
        "/ip4/127.0.0.1/tcp/2/ws/p2p/relay-peer/p2p-circuit/p2p/browser-peer",
        "browser-peer",
      ),
    ).toBe(true);
    expect(
      isExportableBrowserBootstrapAddress(
        "/ip4/127.0.0.1/tcp/2/ws/p2p/relay-peer/p2p-circuit/webrtc/p2p/browser-peer",
        "browser-peer",
      ),
    ).toBe(false);
    expect(isExportableBrowserBootstrapAddress("/webrtc/p2p/browser-peer", "browser-peer")).toBe(
      false,
    );
  });

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
      "/ip4/127.0.0.1/tcp/2/ws/p2p/12D3KooWRelay/p2p-circuit/p2p/12D3KooWNative",
      "/ip4/127.0.0.1/tcp/2/ws/p2p/12D3KooWNative",
    ]);
  });

  it("prioritizes browser target addresses before relay-server hints", () => {
    const record = parseBootstrapRecord({
      peer_id: "browser-peer",
      direct_addresses: [],
      webrtc_direct_addresses: [],
      relay_addresses: ["/ip4/127.0.0.1/tcp/2/ws/p2p/relay-peer/p2p-circuit/p2p/browser-peer"],
      relay_server_addresses: ["/ip4/127.0.0.1/tcp/2/ws/p2p/relay-peer"],
      bootstrap_addresses: [
        "/ip4/127.0.0.1/tcp/2/ws/p2p/relay-peer",
        "/ip4/127.0.0.1/tcp/2/ws/p2p/relay-peer/p2p-circuit/p2p/browser-peer",
      ],
    });

    expect(preferredDialAddresses(record)).toEqual([
      "/ip4/127.0.0.1/tcp/2/ws/p2p/relay-peer/p2p-circuit/p2p/browser-peer",
      "/ip4/127.0.0.1/tcp/2/ws/p2p/relay-peer",
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

  it("parses one or many bootstrap records", () => {
    expect(
      parseBootstrapRecords({
        peer_id: "peer-a",
        direct_addresses: ["/memory/a"],
        webrtc_direct_addresses: [],
        relay_addresses: [],
        relay_server_addresses: [],
      }).map((record) => record.peerId),
    ).toEqual(["peer-a"]);

    expect(
      parseBootstrapRecords([
        {
          peer_id: "peer-a",
          direct_addresses: ["/memory/a"],
          webrtc_direct_addresses: [],
          relay_addresses: [],
          relay_server_addresses: [],
        },
        {
          peer_id: "peer-b",
          direct_addresses: ["/memory/b"],
          webrtc_direct_addresses: [],
          relay_addresses: [],
          relay_server_addresses: [],
        },
      ]).map((record) => record.peerId),
    ).toEqual(["peer-a", "peer-b"]);
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
