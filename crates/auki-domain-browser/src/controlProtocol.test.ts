import { describe, expect, it } from "vitest";
import {
  InfoRequest,
  InfoResponse,
  JoinRequest,
  JoinResponse,
  decodeFrame,
  encodeFrame,
} from "./protocol/control.js";

describe("protobuf control protocol bindings", () => {
  it("encodes join requests as length-prefixed protobuf", () => {
    const request: JoinRequest = {
      multiaddrs: [
        "/ip4/127.0.0.1/tcp/4001/ws/p2p/12D3KooWBrowserPeer",
        "/ip4/127.0.0.1/tcp/5555/ws/p2p/12D3KooWRelay/p2p-circuit/p2p/12D3KooWBrowserPeer",
      ],
    };

    const frame = encodeFrame(JoinRequest.encode(request));
    const decoded = JoinRequest.decode(decodeFrame(frame));

    expect(frame.slice(0, 4)).toEqual(new Uint8Array([0, 0, 0, 134]));
    expect(frame[4]).not.toBe("{".charCodeAt(0));
    expect(decoded).toEqual(request);
  });

  it("round-trips join accept responses", () => {
    const response: JoinResponse = {
      kind: {
        case: "accept",
        value: {
          membershipJson: JSON.stringify({ cluster_name: "demo", peers: [] }),
          successorToken: new Uint8Array([0xde, 0xad, 0xbe, 0xef]),
        },
      },
    };

    const decoded = JoinResponse.decode(decodeFrame(encodeFrame(JoinResponse.encode(response))));

    expect(decoded).toEqual(response);
  });

  it("round-trips info request/response protobufs", () => {
    expect(InfoRequest.decode(InfoRequest.encode({}))).toEqual({});
    expect(InfoResponse.decode(InfoResponse.encode({ participantInfoJson: "{}" }))).toEqual({
      participantInfoJson: "{}",
    });
  });
});
