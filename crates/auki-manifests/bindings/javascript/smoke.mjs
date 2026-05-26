import { readFile } from "node:fs/promises";
import init, {
  buildDetectionLogManifestJson,
  buildPoseLogManifestJson,
  buildSensorLogManifestJson,
  buildTimeTransformLogManifestJson,
  poseSourceRos2TfCanonicalJson,
  poseSourceRos2TfHash,
  timeTransformSourceCanonicalJson,
  timeTransformSourceHash,
} from "./$generated_js_file";

const wasmBytes = await readFile(new URL("./$generated_wasm_file", import.meta.url));
await init({ module_or_path: wasmBytes });

function assert(condition, message) {
  if (!condition) {
    throw new Error(message);
  }
}

const oneSecond = 1000000000n;
const thirtySeconds = 30000000000n;
const sixtySeconds = 60000000000n;
const publishers = ["amcl", "robot_state_publisher", "tf_broadcaster"];

const sensor = JSON.parse(
  buildSensorLogManifestJson(
    "boosterapp",
    "session-1",
    "K1/head",
    "sensorhash",
    "K1/utc",
    "clockhash",
    "K1/head_optical",
    "framehash",
    oneSecond,
    thirtySeconds,
  ),
);
assert(sensor.sensor_id === "K1/head", "sensor manifest vector failed");
assert(sensor.segment_duration_ns === 1000000000, "sensor duration vector failed");

const sourceCanonical = poseSourceRos2TfCanonicalJson(publishers);
assert(
  sourceCanonical ===
    '{"kind":"ros2_tf","publishers":["amcl","robot_state_publisher","tf_broadcaster"]}',
  "pose source canonical vector failed",
);
assert(
  poseSourceRos2TfHash(publishers) === "f3d296341347589c72297a0cc7c81cd8",
  "pose source hash vector failed",
);

const pose = JSON.parse(
  buildPoseLogManifestJson(
    "boosterapp",
    "session-1",
    "map",
    "fromhash",
    "base_link",
    "tohash",
    "K1/utc",
    "clockhash",
    publishers,
    "movable",
    100,
    oneSecond,
    thirtySeconds,
  ),
);
assert(pose.source.kind === "ros2_tf", "pose source kind vector failed");
assert(pose.writer_mode === "movable", "pose writer mode vector failed");

const timeTransform = JSON.parse(
  buildTimeTransformLogManifestJson(
    "boosterapp",
    "session-1",
    "K1/monotonic",
    "fromclockhash",
    "K1/utc",
    "toclockhash",
    "local_clock_read",
    oneSecond,
    sixtySeconds,
  ),
);
assert(timeTransform.source.kind === "local_clock_read", "time transform source vector failed");
assert(
  timeTransformSourceCanonicalJson("local_clock_read") === '{"kind":"local_clock_read"}',
  "time transform canonical vector failed",
);
assert(
  timeTransformSourceHash("local_clock_read") === "8dcea0b9b0b2219d651e0856f112cd65",
  "time transform hash vector failed",
);

const detection = JSON.parse(
  buildDetectionLogManifestJson(
    "boosterapp",
    "session-1",
    "aukilabs/qr/v1",
    "detectorhash",
    "input-log",
    "K1/head",
    "sensorhash",
    "K1/utc",
    "clockhash",
    oneSecond,
    thirtySeconds,
  ),
);
assert(detection.detector_id === "aukilabs/qr/v1", "detection manifest vector failed");
assert(detection.input_log_id === "input-log", "detection input vector failed");

let rejected = false;
try {
  buildPoseLogManifestJson(
    "boosterapp",
    "session-1",
    "map",
    "fromhash",
    "base_link",
    "tohash",
    "K1/utc",
    "clockhash",
    publishers,
    "riged",
    100,
    oneSecond,
    thirtySeconds,
  );
} catch {
  rejected = true;
}
assert(rejected, "invalid writer mode should be rejected");

console.log("javascript wasm smoke ok");
