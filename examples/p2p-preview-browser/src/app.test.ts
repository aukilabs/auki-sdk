import { describe, expect, it } from "vitest";
import {
  bootstrapRecordText,
  canExportLocalBootstrap,
  canRequestSnapshot,
  mergeBootstrapRecords,
  offerLabel,
  parseBootstrapText,
  supportsBrowserToBrowserBootstrap,
} from "./app";
import {
  decodeBase64UrlBytes,
  findPreviewOffer,
  previewFrameBytes,
} from "@aukilabs/auki-p2p-browser";

describe("p2p preview browser helpers", () => {
  it("parses sentinel bootstrap JSON", () => {
    const records = parseBootstrapText(
      JSON.stringify({
        peer_id: "12D3KooWPeer",
        direct_addresses: ["/ip4/127.0.0.1/tcp/40123/ws/p2p/12D3KooWPeer"],
        webrtc_direct_addresses: [
          "/ip4/127.0.0.1/udp/40124/webrtc-direct/certhash/uHash/p2p/12D3KooWPeer",
        ],
        relay_addresses: [],
        relay_server_addresses: ["/ip4/127.0.0.1/tcp/40123/ws/p2p/12D3KooWPeer"],
        bootstrap_addresses: ["/ip4/127.0.0.1/tcp/40123/ws/p2p/12D3KooWPeer"],
      }),
    );
    const [record] = records;

    expect(records).toHaveLength(1);
    expect(record.peerId).toBe("12D3KooWPeer");
    expect(record.webrtcDirectAddresses).toHaveLength(1);
  });

  it("parses multiple sentinel bootstrap records", () => {
    const records = parseBootstrapText(
      JSON.stringify([
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
      ]),
    );

    expect(records.map((record) => record.peerId)).toEqual(["peer-a", "peer-b"]);
  });

  it("merges bootstrap records by peer id", () => {
    const [first, second] = parseBootstrapText(
      JSON.stringify([
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
      ]),
    );
    const [replacement] = parseBootstrapText(
      JSON.stringify([
        {
          peer_id: "peer-a",
          direct_addresses: ["/memory/a2"],
          webrtc_direct_addresses: [],
          relay_addresses: [],
          relay_server_addresses: [],
        },
      ]),
    );

    const merged = mergeBootstrapRecords([first, second], [replacement]);

    expect(merged.map((record) => record.peerId)).toEqual(["peer-a", "peer-b"]);
    expect(merged[0].directAddresses).toEqual(["/memory/a2"]);
  });

  it("serializes local bootstrap records in native-compatible JSON shape", () => {
    expect(
      bootstrapRecordText({
        peerId: "browser-peer",
        agentVersion: "auki-p2p-browser/0.0.0",
        directAddresses: [],
        webrtcDirectAddresses: [],
        relayAddresses: ["/memory/relay/p2p-circuit/p2p/browser-peer"],
        relayServerAddresses: [],
        bootstrapAddresses: ["/memory/relay/p2p-circuit/p2p/browser-peer"],
      }),
    ).toBe(
      JSON.stringify(
        {
          agent_version: "auki-p2p-browser/0.0.0",
          peer_id: "browser-peer",
          direct_addresses: [],
          webrtc_direct_addresses: [],
          relay_addresses: ["/memory/relay/p2p-circuit/p2p/browser-peer"],
          relay_server_addresses: [],
          bootstrap_addresses: ["/memory/relay/p2p-circuit/p2p/browser-peer"],
        },
        null,
        2,
      ),
    );
  });

  it("detects whether the local browser peer is dialable", () => {
    expect(
      canExportLocalBootstrap("browser-peer", [
        "/ip4/127.0.0.1/tcp/1/ws/p2p/relay-peer/p2p-circuit/p2p/browser-peer",
      ]),
    ).toBe(true);
    expect(canExportLocalBootstrap("browser-peer", ["/webrtc/p2p/browser-peer"])).toBe(false);
    expect(canExportLocalBootstrap("browser-peer", [])).toBe(false);
  });

  it("detects browser-to-browser bootstrap addresses", () => {
    expect(
      supportsBrowserToBrowserBootstrap(
        "browser-peer",
        "/ip4/127.0.0.1/tcp/1/ws/p2p/relay-peer/p2p-circuit/p2p/browser-peer",
      ),
    ).toBe(true);
    expect(
      supportsBrowserToBrowserBootstrap(
        "browser-peer",
        "/ip4/127.0.0.1/tcp/1/ws/p2p/relay-peer/p2p-circuit/webrtc/p2p/browser-peer",
      ),
    ).toBe(true);
    expect(
      supportsBrowserToBrowserBootstrap(
        "relay-peer",
        "/ip4/127.0.0.1/tcp/1/ws/p2p/relay-peer",
      ),
    ).toBe(false);
    expect(
      supportsBrowserToBrowserBootstrap(
        "relay-peer",
        "/ip4/127.0.0.1/tcp/1/ws/p2p/relay-peer",
        true,
      ),
    ).toBe(true);
    expect(
      supportsBrowserToBrowserBootstrap("browser-peer", "/webrtc/p2p/browser-peer"),
    ).toBe(false);
  });

  it("selects the shared preview offer profile", () => {
    const offer = findPreviewOffer([
      {
        peerId: "peer-a",
        domainId: "domain-a",
        offerId: "debug",
        kind: "debug",
        payloadType: "text/plain",
        accessModes: ["get"],
      },
      {
        peerId: "peer-b",
        domainId: "domain-b",
        offerId: "sentinel-preview",
        kind: "auki.sensor.rgb_camera.preview",
        payloadType: "auki.camera.jpeg_frame.v1",
        accessModes: ["subscribe"],
      },
    ]);

    expect(offer?.peerId).toBe("peer-b");
    expect(offerLabel(offer)).toBe("peer-b/domain-b/sentinel-preview");
  });

  it("decodes preview payload bytes", () => {
    const bytes = previewFrameBytes({
      type: "auki.spatial_message.v1",
      domain_id: "domain-a",
      offer_id: "sentinel-preview",
      sequence: "1",
      payload: {
        type: "auki.camera.jpeg_frame.v1",
        encoding: "binary",
        bytes: "_9j_2Q",
      },
    });

    expect(Array.from(bytes)).toEqual([255, 216, 255, 217]);
    expect(Array.from(decodeBase64UrlBytes("AQID"))).toEqual([1, 2, 3]);
  });

  it("disables snapshot requests while a stream is active", () => {
    expect(canRequestSnapshot(false, {})).toBe(false);
    expect(canRequestSnapshot(true, {})).toBe(true);
    expect(canRequestSnapshot(true, { getting: true })).toBe(false);
    expect(canRequestSnapshot(true, { subscribing: true })).toBe(false);
    expect(canRequestSnapshot(true, { stopping: true })).toBe(false);
    expect(canRequestSnapshot(true, { subscription: {} })).toBe(false);
  });
});
