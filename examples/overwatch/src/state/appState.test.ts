import { describe, expect, it } from "vitest";

import { deriveOperatorState } from "./appState";
import type { PeerSnapshot } from "../sdk/contract";

describe("deriveOperatorState", () => {
  it("splits self from remotes and reports the empty remote state", () => {
    const snapshot: PeerSnapshot = {
      selfPeerId: "peer-a",
      domainName: "overwatch",
      managerPeerId: "peer-a",
      role: "manager",
      participants: [
        {
          peer_id: "peer-a",
          name: "Browser A",
          app: "overwatch",
          is_self: true,
          is_manager: true,
          connected: true,
          sensors: [],
        },
      ],
    };

    expect(deriveOperatorState(snapshot)).toMatchObject({
      self: { peer_id: "peer-a" },
      remotes: [],
      banner: { kind: "empty", text: "No remote peers" },
    });
  });
});
