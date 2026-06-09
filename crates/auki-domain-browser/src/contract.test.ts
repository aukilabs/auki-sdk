import { describe, expect, it } from "vitest";
import { SDK_SENSOR_KINDS, SDK_STREAM_STATES } from "./contract.js";
import type { SensorKind, StreamState } from "./contract.js";

describe("browser domain contract vocabulary", () => {
  it("uses the current SDK sensor kinds", () => {
    expect(SDK_SENSOR_KINDS).toEqual([
      "camera",
      "point_cloud",
      "joint_encoders",
      "audio",
    ]);

    const kind: SensorKind = "audio";
    expect(kind).toBe("audio");
  });

  it("uses UI-friendly SDK stream states", () => {
    expect(SDK_STREAM_STATES).toEqual([
      "off",
      "idle",
      "connecting",
      "connected",
      "reconnecting",
      "declined",
      "error",
    ]);

    const state: StreamState = "declined";
    expect(state).toBe("declined");
  });
});
