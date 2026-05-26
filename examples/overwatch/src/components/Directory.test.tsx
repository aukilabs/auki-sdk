import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { Directory } from "./Directory";
import type { PeerSnapshot } from "../sdk/contract";

describe("Directory", () => {
  it("shows self, remotes, manager state, and stream health", () => {
    render(<Directory snapshot={snapshotWithTwoPeers()} selectedPeerId={null} onSelectPeer={() => {}} />);

    expect(screen.getByText("you")).toBeInTheDocument();
    expect(screen.getByText("Manager")).toBeInTheDocument();
    expect(screen.getByText("browser-a/audio")).toBeInTheDocument();
    expect(screen.getByText("1 remote")).toBeInTheDocument();
  });
});

function snapshotWithTwoPeers(): PeerSnapshot {
  return {
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
        sensors: [
          {
            sensor_id: "browser-a/audio",
            sensor_hash: "audio-hash",
            kind: "audio",
            label: "Microphone",
          },
        ],
      },
      {
        peer_id: "peer-b",
        name: "Browser B",
        app: "overwatch",
        connected: true,
        sensors: [],
      },
    ],
  };
}
