import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { App, defaultDiscoveryUrlForProtocol } from "./App";
import type { OverwatchPeer, PeerSnapshot, StreamHandle } from "./sdk/contract";

const { createOverwatchPeer } = vi.hoisted(() => ({
  createOverwatchPeer: vi.fn<() => Promise<OverwatchPeer>>(),
}));

vi.mock("./sdk/createOverwatchPeer", () => ({
  createOverwatchPeer,
}));

describe("App", () => {
  beforeEach(() => {
    createOverwatchPeer.mockReset();
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("renders the operator shell as the first screen", () => {
    render(<App />);

    expect(screen.getByRole("banner")).toHaveTextContent("Auki Overwatch");
    expect(screen.getByLabelText(/Discovery URL/i)).toHaveValue("http://127.0.0.1:8091");
    expect(screen.getByLabelText(/Domain name/i)).toHaveValue("overwatch");
    expect(screen.getByText("No remote peers")).toBeInTheDocument();
  });

  it("uses a same-origin Discovery proxy when served over HTTPS", () => {
    expect(defaultDiscoveryUrlForProtocol("https:")).toBe("/discovery");
    expect(defaultDiscoveryUrlForProtocol("http:")).toBe("http://127.0.0.1:8091");
  });

  it("keeps reading subscribed remote camera streams", async () => {
    const createObjectURL = vi
      .spyOn(URL, "createObjectURL")
      .mockReturnValueOnce("blob:frame-1")
      .mockReturnValueOnce("blob:frame-2");
    const secondEntry = deferred<unknown>();
    const peer = fakePeer({
      nextMessage: vi
        .fn()
        .mockResolvedValueOnce({ accept: { sensor_id: "remote-camera" } })
        .mockResolvedValueOnce({ entry: { payload: [255, 216, 1], seq: 1 } })
        .mockImplementationOnce(() => secondEntry.promise)
        .mockResolvedValue(null),
    });
    createOverwatchPeer.mockResolvedValue(peer);

    render(<App />);

    await userEvent.click(screen.getByRole("button", { name: /join domain/i }));
    await userEvent.click(await screen.findByRole("button", { name: /remote browser/i }));
    await userEvent.click(screen.getByRole("button", { name: "Webcam" }));

    const image = await screen.findByRole("img", { name: /camera frame/i });
    expect(image).toHaveAttribute("src", "blob:frame-1");
    secondEntry.resolve({ entry: { payload: [255, 216, 2], seq: 2 } });
    await waitFor(() => expect(image).toHaveAttribute("src", "blob:frame-2"));
    expect(peer.subscribeToSensor).toHaveBeenCalledWith("remote-peer", "remote-camera");
    createObjectURL.mockRestore();
  });
});

function fakePeer(stream: StreamHandle): OverwatchPeer {
  return {
    peerId: "self-peer",
    createOrJoin: vi.fn(),
    observeParticipants: vi.fn((callback: (snapshot: PeerSnapshot) => void) => {
      callback(fakeSnapshot());
      return () => {};
    }),
    declareSensors: vi.fn(),
    publishSensor: vi.fn(),
    subscribeToSensor: vi.fn(async () => stream),
    debugState: vi.fn(() => ({})),
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((settle) => {
    resolve = settle;
  });
  return { promise, resolve };
}

function fakeSnapshot(): PeerSnapshot {
  return {
    selfPeerId: "self-peer",
    domainName: "overwatch",
    managerPeerId: "self-peer",
    role: "manager",
    participants: [
      {
        peer_id: "self-peer",
        name: "This browser",
        is_self: true,
        is_manager: true,
        connected: true,
        sensors: [],
      },
      {
        peer_id: "remote-peer",
        name: "Remote browser",
        is_self: false,
        connected: true,
        sensors: [
          {
            sensor_id: "remote-camera",
            sensor_hash: "remote-camera-hash",
            kind: "camera",
            label: "Webcam",
          },
        ],
      },
    ],
  };
}
