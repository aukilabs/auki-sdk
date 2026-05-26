import { readFile } from "node:fs/promises";
import init, { canonicalizeJson } from "./$generated_js_file";

const wasmBytes = await readFile(new URL("./$generated_wasm_file", import.meta.url));
await init({ module_or_path: wasmBytes });

function assert(condition, message) {
  if (!condition) {
    throw new Error(message);
  }
}

const canonical = new TextDecoder().decode(canonicalizeJson('{"b":2,"a":1}'));
assert(canonical === '{"a":1,"b":2}', "canonical object vector failed");

const slash = new TextDecoder().decode(canonicalizeJson('{"path":"a/b"}'));
assert(slash === '{"path":"a/b"}', "forward slash vector failed");

let rejected = false;
try {
  canonicalizeJson("{");
} catch {
  rejected = true;
}
assert(rejected, "invalid JSON should be rejected");

console.log("javascript wasm smoke ok");
