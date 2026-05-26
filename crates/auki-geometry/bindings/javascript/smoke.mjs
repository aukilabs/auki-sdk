import { readFile } from "node:fs/promises";
import init, {
  axisConventionMatrixJson,
  convertDirectionConventionJson,
  convertPointConventionJson,
  convertPoseConventionJson,
  metersPerUnitJson,
} from "./$generated_js_file";

const wasmBytes = await readFile(new URL("./$generated_wasm_file", import.meta.url));
await init({ module_or_path: wasmBytes });

function assert(condition, message) {
  if (!condition) {
    throw new Error(message);
  }
}

const rosOptical = {
  frame_id: "camera",
  handedness: "right",
  axes: { x: "right", y: "down", z: "forward" },
  units: "centimeters",
};
const opengl = {
  frame_id: "world",
  handedness: "right",
  axes: { x: "right", y: "up", z: "backward" },
  units: "meters",
};

assert(metersPerUnitJson("centimeters") === 0.01, "unit vector failed");
assert(
  axisConventionMatrixJson(JSON.stringify(rosOptical.axes), JSON.stringify(opengl.axes)) ===
    "[[1.0,0.0,0.0],[0.0,-1.0,0.0],[0.0,0.0,-1.0]]",
  "axis matrix vector failed",
);
assert(
  convertPointConventionJson(
    JSON.stringify({ x: 100.0, y: 200.0, z: 300.0 }),
    JSON.stringify(rosOptical),
    JSON.stringify(opengl),
  ) === '{"x":1.0,"y":-2.0,"z":-3.0}',
  "point conversion vector failed",
);
assert(
  convertDirectionConventionJson(
    JSON.stringify({ x: 1.0, y: 2.0, z: 3.0 }),
    JSON.stringify(rosOptical),
    JSON.stringify(opengl),
  ) === '{"x":1.0,"y":-2.0,"z":-3.0}',
  "direction conversion vector failed",
);
assert(
  convertPoseConventionJson(
    JSON.stringify({ translation: { x: 1.0, y: 2.0, z: 3.0 } }),
    JSON.stringify({ ...rosOptical, units: "meters" }),
    JSON.stringify(opengl),
  ) === '{"orientation":null,"translation":{"x":1.0,"y":-2.0,"z":-3.0}}',
  "pose conversion vector failed",
);

let rejected = false;
try {
  convertPointConventionJson(
    JSON.stringify({ x: 1.0, y: 2.0, z: 3.0 }),
    JSON.stringify({ ...opengl, handedness: "left" }),
    JSON.stringify(opengl),
  );
} catch {
  rejected = true;
}
assert(rejected, "handedness mismatch should be rejected");

console.log("javascript wasm smoke ok");
