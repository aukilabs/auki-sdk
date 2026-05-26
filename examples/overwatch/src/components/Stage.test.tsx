import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { Stage } from "./Stage";

describe("Stage", () => {
  it("renders one tile per toggled sensor and closes tiles independently", async () => {
    render(
      <Stage
        tiles={[
          { kind: "camera", peerId: "peer-a", sensorId: "camera" },
          { kind: "audio", peerId: "peer-a", sensorId: "audio" },
        ]}
      />,
    );

    expect(screen.getAllByTestId("stage-tile")).toHaveLength(2);
    await userEvent.click(screen.getByRole("button", { name: /close audio/i }));
    expect(screen.getAllByTestId("stage-tile")).toHaveLength(1);
  });

  it("renders camera stream JPEG payloads as an image tile", () => {
    const createObjectURL = vi
      .spyOn(URL, "createObjectURL")
      .mockReturnValue("blob:camera-frame");

    render(
      <Stage
        tiles={[
          {
            kind: "camera",
            peerId: "peer-a",
            sensorId: "camera",
            latestMessage: { entry: { payload: [255, 216, 255], seq: 4 } },
          },
        ]}
      />,
    );

    const image = screen.getByRole("img", { name: /camera frame/i });
    expect(image).toHaveAttribute("src", "blob:camera-frame");
    createObjectURL.mockRestore();
  });
});
