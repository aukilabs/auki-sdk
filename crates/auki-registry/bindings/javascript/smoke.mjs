import { readFile } from "node:fs/promises";
import init, {
  clockEntryHash,
  detectorEntryCanonicalJson,
  frameEntryHash,
  frameRosBodyJson,
  sensorEntryHash,
} from "./$generated_js_file";

const wasmBytes = await readFile(new URL("./$generated_wasm_file", import.meta.url));
await init({ module_or_path: wasmBytes });

function assert(condition, message) {
  if (!condition) {
    throw new Error(message);
  }
}

const frameJson = frameRosBodyJson("K1-AABBCCDDEEFF/base_link");
assert(
  frameJson ===
    '{"axes":{"x":"forward","y":"left","z":"up"},"frame_id":"K1-AABBCCDDEEFF/base_link","handedness":"right","units":"meters"}',
  "frame preset canonical vector failed",
);
assert(
  frameEntryHash(frameJson) === "fd0dc3789e898b71b5e16ee122a81a44",
  "frame hash vector failed",
);

const sensorJson = JSON.stringify({
  sensor_id: "K1-AABBCCDDEEFF/head_left_cam",
  type: "camera",
  width: 544,
  height: 488,
  frame_rate_hz: 20,
  pixel_format: "YUV_NV12",
  color_space: "BT.709",
  intrinsics_model: "pinhole",
  distortion_model: "plumb_bob",
  frame_id: "K1-AABBCCDDEEFF/head_left_cam_optical",
  frame_hash: "e0d40e7b526e04f15f83f75897f53825",
});
assert(
  sensorEntryHash(sensorJson) === "5559c9648e31eee2410b692fef393489",
  "sensor hash vector failed",
);

const clockJson = JSON.stringify({
  clock_id: "K1-AABBCCDDEEFF/utc",
  type: "utc_clock",
  unit: "milliseconds",
  monotonic: false,
  epoch: "1970-01-01T00:00:00Z",
  scope: "global",
});
assert(
  clockEntryHash(clockJson) === "89f84f4c2e09bef81d385b2af1d17e6c",
  "clock hash vector failed",
);

const detectorJson = JSON.stringify({
  detector_id: "aukilabs/aruco/v1",
  type: "aruco",
  dictionary: "5x5_50",
  output_types: ["aruco"],
});
assert(
  detectorEntryCanonicalJson(detectorJson) ===
    '{"detector_id":"aukilabs/aruco/v1","dictionary":"5x5_50","output_types":["aruco"],"type":"aruco"}',
  "detector canonical vector failed",
);

let rejected = false;
try {
  sensorEntryHash('{"sensor_id":"bad","type":"not_a_sensor"}');
} catch {
  rejected = true;
}
assert(rejected, "invalid sensor entry should be rejected");

console.log("javascript wasm smoke ok");
