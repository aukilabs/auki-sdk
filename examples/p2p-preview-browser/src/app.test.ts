import { describe, expect, it } from "vitest";
import {
  decodeBase64Url,
  findPreviewOffer,
  offerLabel,
  parseBootstrapText,
  previewFrameBytes,
} from "./app";

describe("p2p preview browser helpers", () => {
  it("parses sentinel bootstrap JSON", () => {
    const record = parseBootstrapText(
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

    expect(record.peerId).toBe("12D3KooWPeer");
    expect(record.webrtcDirectAddresses).toHaveLength(1);
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
    expect(offerLabel(offer)).toBe("domain-b/sentinel-preview");
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
    expect(Array.from(decodeBase64Url("AQID"))).toEqual([1, 2, 3]);
  });
});
