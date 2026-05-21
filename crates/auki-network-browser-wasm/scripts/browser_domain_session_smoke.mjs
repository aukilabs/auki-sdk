import sdk from "../pkg-node/auki_network_browser_wasm.js";

const { BrowserDomainSession } = sdk;

const session = new BrowserDomainSession(new Uint8Array(32).fill(3));
const peerId = session.peerId();

if (peerId !== "12D3KooWAvnEo4RaYZtqt2w83qzmQ7WVW2HhN2cay95EXAiVKcar") {
  throw new Error(`bad peer id: ${peerId}`);
}

console.log(`ok ${peerId}`);
