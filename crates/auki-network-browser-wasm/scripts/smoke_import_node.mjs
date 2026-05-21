import { peerIdFromSeed, sdkName } from "../pkg-node/auki_network_browser_wasm.js";

const seed = new Uint8Array(32).fill(3);
const peerId = peerIdFromSeed(seed);

if (sdkName() !== "auki-network-browser-wasm") {
  throw new Error(`unexpected sdkName: ${sdkName()}`);
}

if (peerId !== "12D3KooWAvnEo4RaYZtqt2w83qzmQ7WVW2HhN2cay95EXAiVKcar") {
  throw new Error(`unexpected peer id: ${peerId}`);
}

console.log(`ok ${peerId}`);
