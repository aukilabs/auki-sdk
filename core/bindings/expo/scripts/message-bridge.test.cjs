const assert = require("node:assert/strict");
const Module = require("node:module");
const ts = require("typescript");

require.extensions[".ts"] = (loaded, filename) => {
  const source = require("node:fs").readFileSync(filename, "utf8");
  loaded._compile(ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020, esModuleInterop: true },
  }).outputText, filename);
};

const bridge = {};
const originalLoad = Module._load;
Module._load = function load(request, parent, isMain) {
  if (request === "expo") return { NativeModule: class {}, requireNativeModule: () => bridge };
  return originalLoad.call(this, request, parent, isMain);
};

const { AukiMessageEndpoint, AukiMessageSender, MESSAGE_PROTOCOL_ID } = require("../src/Message.ts");

const channel = {
  owner_peer_id: "12D3KooWH3okqZcRaHwy4keYWo9eAaCDwhePYajtHsCM4Egsptan",
  resource_id: "ble-tracker/session/abc",
  clock: { peer_id: "12D3KooWH3okqZcRaHwy4keYWo9eAaCDwhePYajtHsCM4Egsptan", id: "monotonic", hash: "h" },
};
const sender = {
  peerId: "12D3KooWRemote", subject: "user-1", domainIds: ["domain"], scopes: [], verifiedUntil: "2026-10-07T00:00:00Z",
};

function event(payload) {
  return JSON.stringify({
    channel, sender, messageType: "obs.v1", timestampNs: "9223372036854775807",
    payloadBase64: Buffer.from(payload).toString("base64"),
  });
}

async function main() {
  assert.equal(MESSAGE_PROTOCOL_ID, "/auki/auth/1/message/0.1.0");

  bridge.messageMount = async peer => { assert.equal(peer, "peer"); return "endpoint"; };
  const endpoint = await AukiMessageEndpoint.mount("peer");

  bridge.messageDeclare = async (handle, channelJson, capacity) => {
    assert.equal(handle, "endpoint");
    assert.deepEqual(JSON.parse(channelJson), channel);
    assert.equal(capacity, 256);
    return "receiver";
  };
  const receiver = await endpoint.declare(channel);
  assert.deepEqual(receiver.channel, channel);

  // Timestamps stay strings so i64 precision survives; payload decodes to bytes.
  const queue = [event("hello"), event(""), null];
  bridge.messageNext = async handle => { assert.equal(handle, "receiver"); return queue.shift(); };
  const first = await receiver.next();
  assert.equal(first.messageType, "obs.v1");
  assert.equal(first.timestampNs, "9223372036854775807");
  assert.equal(Buffer.from(first.payload).toString(), "hello");
  assert.equal(first.sender.subject, "user-1");
  const rest = [];
  for await (const message of receiver) rest.push(message);
  assert.equal(rest.length, 1);
  assert.equal(rest[0].payload.length, 0);

  // Only one next() may be pending, matching the native receiver contract.
  let release;
  bridge.messageNext = () => new Promise(resolve => { release = resolve; });
  const pending = receiver.next();
  await assert.rejects(receiver.next(), /already pending/);
  release(null);
  assert.equal(await pending, null);

  let receiverClosed = 0;
  bridge.messageReceiverClose = async handle => { assert.equal(handle, "receiver"); receiverClosed += 1; };
  await receiver.close();
  await receiver.close();
  assert.equal(receiverClosed, 1);
  assert.equal(await receiver.next(), null);

  let endpointClosed = 0;
  bridge.messageEndpointClose = async handle => { assert.equal(handle, "endpoint"); endpointClosed += 1; };
  await endpoint.close();
  await endpoint.close();
  assert.equal(endpointClosed, 1);
  await assert.rejects(endpoint.declare(channel), /closed/);

  bridge.messageOpenExact = async (peer, target, channelJson) => {
    assert.equal(peer, "peer");
    assert.deepEqual(target, { peerId: "remote", route: "/ip4/127.0.0.1/tcp/1" });
    assert.deepEqual(JSON.parse(channelJson), channel);
    return "sender";
  };
  const out = await AukiMessageSender.open("peer", { peerId: "remote", route: "/ip4/127.0.0.1/tcp/1" }, channel);
  bridge.messageSend = async (handle, type, timestampNs, payloadBase64) => {
    assert.equal(handle, "sender");
    assert.equal(type, "obs.v1");
    assert.equal(timestampNs, "123");
    assert.equal(Buffer.from(payloadBase64, "base64").toString(), "hi");
  };
  await out.send("obs.v1", 123n, new TextEncoder().encode("hi"));
  bridge.messageClose = async handle => { assert.equal(handle, "sender"); };
  await out.close();
  await assert.rejects(out.send("obs.v1", "1", new Uint8Array()), /closed/);

  console.log("message bridge tests passed");
}

main().catch(error => {
  console.error(error);
  process.exit(1);
});
