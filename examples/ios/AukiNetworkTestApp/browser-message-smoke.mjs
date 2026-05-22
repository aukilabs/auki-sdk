#!/usr/bin/env node
import { readFile } from "node:fs/promises";

function usage() {
  console.log("Usage: node examples/ios/AukiNetworkTestApp/browser-message-smoke.mjs <ios-multiaddr>");
}

const target = process.argv[2];
if (target == null || target === "--help") {
  usage();
  process.exit(target === "--help" ? 0 : 1);
}

const {
  default: initAukiNetwork,
  createAukiNetworkPeer,
  messageProtocol,
} = await import(new URL("../../../bindings/javascript/auki-network/index.js", import.meta.url));

const wasmBytes = await readFile(
  new URL("../../../bindings/javascript/auki-network/auki_network_bg.wasm", import.meta.url),
);
await initAukiNetwork({ module_or_path: wasmBytes });

const walletSeed = new Uint8Array(32);
walletSeed.fill(4);
const peer = await createAukiNetworkPeer({ walletSeed });

try {
  const stream = await peer.dialProtocol(target, messageProtocol());
  const requestId = `browser-${Date.now()}`;
  const payload = encodeEnvelope({
    typeUrl: "auki.test/ping",
    body: new TextEncoder().encode("hello from browser"),
    requestId,
  });

  const len = new Uint8Array(4);
  new DataView(len.buffer).setUint32(0, payload.length, false);
  await stream.sink([len, payload]);

  console.log(`sent ${requestId} to ${target}`);
} finally {
  await peer.stop();
}

function encodeEnvelope({ typeUrl, body, requestId }) {
  return concat([
    fieldBytes(1, new TextEncoder().encode(typeUrl)),
    fieldBytes(2, body),
    fieldBytes(3, new TextEncoder().encode(requestId)),
  ]);
}

function fieldBytes(fieldNo, bytes) {
  return concat([
    varint((fieldNo << 3) | 2),
    varint(bytes.length),
    bytes,
  ]);
}

function varint(value) {
  const out = [];
  let n = value >>> 0;
  while (n >= 0x80) {
    out.push((n & 0x7f) | 0x80);
    n >>>= 7;
  }
  out.push(n);
  return Uint8Array.from(out);
}

function concat(chunks) {
  const len = chunks.reduce((sum, chunk) => sum + chunk.length, 0);
  const out = new Uint8Array(len);
  let offset = 0;
  for (const chunk of chunks) {
    out.set(chunk, offset);
    offset += chunk.length;
  }
  return out;
}
