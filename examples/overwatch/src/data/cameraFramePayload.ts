import { fromBinary } from "@bufbuild/protobuf";
import { CameraFrameSchema } from "@aukilabs/auki-proto/src/auki/camera_pb.js";

export function previewPayloadBytes(payload: Uint8Array, sensorKind?: string): Uint8Array | null {
  if (sensorKind !== "camera" || isJpeg(payload)) {
    return payload;
  }

  try {
    const frame = fromBinary(CameraFrameSchema, payload);
    return frame.frame.length > 0 ? frame.frame : null;
  } catch {
    return null;
  }
}

function isJpeg(payload: Uint8Array): boolean {
  return payload[0] === 0xff && payload[1] === 0xd8;
}
