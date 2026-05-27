import { describe, expect, it } from "vitest";
import { sensorTypeFromCatalog } from "./icons";

describe("sensorTypeFromCatalog", () => {
  it("uses catalog kind when the sensor id does not name the modality", () => {
    expect(sensorTypeFromCatalog("K1-WALK01/stereonet", "point_cloud")).toBe(
      "pointcloud",
    );
  });
});
