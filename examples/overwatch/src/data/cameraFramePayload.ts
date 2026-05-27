import { fromBinary } from "@bufbuild/protobuf";
import { CameraFrameSchema } from "@aukilabs/auki-proto/src/auki/camera_pb.js";

export function previewPayloadBytes(payload: Uint8Array, sensorKind?: string): Uint8Array {
  if (sensorKind !== "camera") {
    return payload;
  }

  const frame = fromBinary(CameraFrameSchema, payload);
  if (frame.frame.length === 0) {
    throw new Error("CameraFrame contained an empty frame field");
  }
  return frame.frame;
}
