import { readFile } from "node:fs/promises";
import init, { hashJcsBytes } from "./$generated_js_file";

const wasmBytes = await readFile(new URL("./$generated_wasm_file", import.meta.url));
await init({ module_or_path: wasmBytes });

function assert(condition, message) {
  if (!condition) {
    throw new Error(message);
  }
}

const abc = new TextEncoder().encode("abc");
const empty = new Uint8Array();

assert(hashJcsBytes(empty) === "99aa06d3014798d86001c324468d497f", "empty vector failed");
assert(hashJcsBytes(abc) === "06b05ab6733a618578af5f94892f3950", "abc vector failed");

console.log("javascript wasm smoke ok");
