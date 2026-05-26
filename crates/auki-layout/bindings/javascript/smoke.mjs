import { readFile } from "node:fs/promises";
import init, {
  detectionLogPath,
  idToSegment,
  registriesRoot,
  sensorEntryPath,
} from "./$generated_js_file";

const wasmBytes = await readFile(new URL("./$generated_wasm_file", import.meta.url));
await init({ module_or_path: wasmBytes });

function assert(condition, message) {
  if (!condition) {
    throw new Error(message);
  }
}

assert(registriesRoot("/app") === "/app/registries", "registries root vector failed");
assert(
  sensorEntryPath("/app", "K1/head", "abcd") ===
    "/app/registries/sensors/K1__head/abcd.json",
  "sensor entry vector failed",
);
assert(
  detectionLogPath("/app/session-1", "aukilabs/qr/v1", "input-log") ===
    "/app/session-1/detection_logs/aukilabs__qr__v1__input-log",
  "detection log vector failed",
);
assert(idToSegment("a/b/c") === "a__b__c", "id segment vector failed");

console.log("javascript wasm smoke ok");
