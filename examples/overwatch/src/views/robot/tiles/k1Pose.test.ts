import { describe, expect, it } from "vitest";
import { k1PoseDropDelta } from "./k1Pose";

describe("k1PoseDropDelta", () => {
  it("does not treat non-contiguous producer sequence numbers as rendered pose drops", () => {
    expect(k1PoseDropDelta(100, 140)).toBe(0);
  });
});
