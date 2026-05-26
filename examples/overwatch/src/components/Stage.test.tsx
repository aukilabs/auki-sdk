import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";

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
});
