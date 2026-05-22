import { readFile } from "node:fs/promises";
import init, {
  Wallet,
  verify,
  loadOrMintSeed,
} from "./$generated_js_file";

const wasmBytes = await readFile(new URL("./$generated_wasm_file", import.meta.url));
await init({ module_or_path: wasmBytes });

function assert(condition, message) {
  if (!condition) {
    throw new Error(message);
  }
}

function bytesEqual(left, right) {
  return left.length === right.length && left.every((value, index) => value === right[index]);
}

const seed = new Uint8Array(32);
seed.fill(3);
const wallet = Wallet.fromSeed(seed);
assert(bytesEqual(wallet.seed(), seed), "seed round-trip failed");
assert(wallet.id().length === 32, "wallet id shape failed");

const msg = new TextEncoder().encode("hello, auki");
const signature = wallet.sign(msg);
assert(signature.length === 64, "signature shape failed");
verify(wallet.publicKey(), msg, signature);

const tampered = new TextEncoder().encode("hello, tampered");
let rejected = false;
try {
  verify(wallet.publicKey(), tampered, signature);
} catch {
  rejected = true;
}
assert(rejected, "tampered message verification should fail");

const child = wallet.deriveChild("peer/v1");
const expectedChildPubkey = Uint8Array.from([
  0x10, 0x80, 0x63, 0x3b, 0xcb, 0x57, 0xba, 0xc0,
  0x66, 0xcf, 0x84, 0x46, 0xe2, 0xb7, 0xae, 0x71,
  0x15, 0x71, 0xcb, 0x04, 0xbe, 0x0b, 0x46, 0xbd,
  0xaf, 0x03, 0x14, 0x63, 0x17, 0xbf, 0xe7, 0x07,
]);
assert(bytesEqual(child.publicKey(), expectedChildPubkey), "deriveChild vector failed");

const signed = wallet.signCanonicalJson(JSON.stringify({ b: 2, a: 1 }));
const canonical = new TextDecoder().decode(signed.canonicalBytes);
assert(canonical === '{"a":1,"b":2}', "canonical JSON failed");
assert(signed.signature.length === 64, "canonical signature shape failed");

const cert = wallet.issueCreationCert(child, "app:test", 42n);
cert.verify();

let storageRejected = false;
try {
  loadOrMintSeed("auki.identity.test.seed");
} catch {
  storageRejected = true;
}
assert(storageRejected, "loadOrMintSeed should report unavailable storage in Node");

if (typeof cert.free === "function") cert.free();
if (typeof signed.free === "function") signed.free();
if (typeof child.free === "function") child.free();
if (typeof wallet.free === "function") wallet.free();

console.log("javascript wasm smoke ok");
