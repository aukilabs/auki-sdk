import { readFile } from "node:fs/promises";
import initAukiNetwork, {
  browserProbeProtocol,
  decodeBrowserProbeResponse,
  encodeBrowserProbeRequest,
  peerDerivationLabel,
  peerIdFromWalletSeed,
  peerPrivateKeyProtobufFromWalletSeed,
} from "./index.js";

const wasmBytes = await readFile(new URL("./$generated_wasm_file", import.meta.url));
await initAukiNetwork({ module_or_path: wasmBytes });

function assert(condition, message) {
  if (!condition) {
    throw new Error(message);
  }
}

const walletSeed = new Uint8Array(32);
walletSeed.fill(3);

assert(peerDerivationLabel() === "peer/v1", "peer derivation label drifted");
assert(
  peerIdFromWalletSeed(walletSeed) === "12D3KooWAvnEo4RaYZtqt2w83qzmQ7WVW2HhN2cay95EXAiVKcar",
  "wallet seed to PeerId vector drifted"
);

const privateKey = peerPrivateKeyProtobufFromWalletSeed(walletSeed);
assert(privateKey.length > 32, "private key protobuf shape failed");

const requestBytes = encodeBrowserProbeRequest("probe-1", new Uint8Array([1, 2, 3]));
const request = JSON.parse(new TextDecoder().decode(requestBytes));
assert(request.nonce === "probe-1", "browser probe request nonce failed");
assert(request.payload.length === 3, "browser probe request payload failed");

const responseBytes = new TextEncoder().encode(JSON.stringify({
  nonce: "probe-1",
  payload: [1, 2, 3],
  responder: "native:test",
}));
const response = JSON.parse(decodeBrowserProbeResponse(responseBytes));
assert(response.responder === "native:test", "browser probe response decode failed");

assert(browserProbeProtocol() === "/auki/browser-probe/0.0.1", "browser probe protocol drifted");

console.log("javascript wasm smoke ok");
