import { readFile } from "node:fs/promises";
import init, {
  canonicalManifestJson,
  decodeSegmentEntriesJson,
  encodeSegmentEntriesJson,
} from "./$generated_js_file";

const wasmBytes = await readFile(new URL("./$generated_wasm_file", import.meta.url));
await init({ module_or_path: wasmBytes });

function assert(condition, message) {
  if (!condition) {
    throw new Error(message);
  }
}

const manifest = canonicalManifestJson(
  JSON.stringify({
    retention_ns: 3000000000,
    kind: "test",
    segment_duration_ns: 1000000000,
  }),
);
assert(
  manifest ===
    '{"kind":"test","retention_ns":3000000000,"segment_duration_ns":1000000000}',
  "manifest canonical vector failed",
);

const entries = JSON.stringify([
  { timestamp_ns: 100, payload_hex: "010203" },
  { timestamp_ns: 200, payload_hex: "68656c6c6f" },
]);
const segment = encodeSegmentEntriesJson(0n, entries);
assert(segment[0] === 0x41 && segment[1] === 0x4b, "segment magic vector failed");
assert(segment[4] === 1 && segment[5] === 0, "segment version vector failed");
assert(
  decodeSegmentEntriesJson(segment) ===
    '[{"payload_hex":"010203","timestamp_ns":100},{"payload_hex":"68656c6c6f","timestamp_ns":200}]',
  "segment decode vector failed",
);

let rejected = false;
try {
  encodeSegmentEntriesJson(0n, JSON.stringify([{ timestamp_ns: 1, payload_hex: "abc" }]));
} catch {
  rejected = true;
}
assert(rejected, "invalid hex should be rejected");

console.log("javascript wasm smoke ok");
