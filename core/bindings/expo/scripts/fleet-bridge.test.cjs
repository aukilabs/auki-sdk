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


const { fleet } = require("../src/Fleet.ts");

async function main() {
  bridge.fleetOpen = async (session, domain) => {
    assert.equal(session, "session"); assert.equal(domain, "domain"); return "fleet-client";
  };
  const client = await fleet("session", "domain");
  bridge.fleetList = async (_client, query) => {
    assert.deepEqual(JSON.parse(query), { capabilities: ["vendor/v7"], match_all_capabilities: true });
    return JSON.stringify({ view: "domain", complete: false, machines: [{kind: "robot", work_state: "unknown"}], sources: [{ source: "busy", state: "unsupported" }] });
  };
  const partial = await client.list({ capabilities: ["vendor/v7"], matchAllCapabilities: true });
  assert.equal(partial.machines[0].work_state, "unknown");
  assert.equal(partial.sources[0].state, "unsupported");
  bridge.fleetComputePool = async (_client, query) => {
    assert.deepEqual(JSON.parse(query), { mode: "dedicated" });
    return JSON.stringify({ view: "compute_pool", machines: [{ association: "candidate", last_seen_at: null }] });
  };
  assert.equal((await client.computePool({ mode: "dedicated" })).machines[0].last_seen_at, null);
  bridge.fleetList = async () => { throw Object.assign(new Error("save failed"), { code: "fleet:auth::persistence" }); };
  await assert.rejects(client.list(), error => error.kind === "auth" && error.code === "persistence");
  bridge.fleetList = async () => { throw Object.assign(new Error("invalid response"), { kind: "response", code: "invalid_response" }); };
  await assert.rejects(client.list(), error => error.kind === "response" && error.code === "invalid_response");
  const beforeStart = new Set();
  bridge.fleetOperationCancel = async id => { beforeStart.add(id); };
  bridge.fleetList = async (_client, _query, id) => {
    assert(beforeStart.delete(id));
    throw Object.assign(new Error("cancelled"), { code: "fleet:cancelled::cancelled" });
  };
  const controller = new AbortController();
  const pending = client.list({}, controller.signal);
  controller.abort();
  await assert.rejects(pending, error => error.kind === "cancelled");
  await assert.rejects(client.list({}, controller.signal), error => error.kind === "cancelled");
  let release;
  bridge.fleetClose = () => new Promise(resolve => { release = resolve; });
  const closing = client.close();
  assert.equal(client.close(), closing);
  await assert.rejects(client.list(), error => error.kind === "closed");
  let settled = false;
  void closing.then(() => { settled = true; });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(settled, false);
  release();
  await closing;
  await client.close();
  console.log("PASS Expo fleet bridge tests");
}
async function webAdapter() {
  const previousLoad = Module._load;
  Module._load = function load(request, parent, isMain) {
    if (request === "expo") return { NativeModule: class {}, registerWebModule: type => new type() };
    if (request === "./web/loadAukiSdkWasm") return { loadAukiSdkWasm: async () => { throw Error("unexpected peer/runtime initialization"); } };
    return previousLoad.call(this, request, parent, isMain);
  };
  const web = require("../src/AukiSdkExpoModule.web.ts").default;
  let received;
  let freed = 0;
  let release;
  const native = {
    list: async (query, signal) => {
      received = query;
      if (signal.aborted) throw Object.assign(new Error("cancelled"), { kind: "cancelled", code: "cancelled" });
      return { view: "domain", machines: [], sources: [] };
    },
    computePool: async query => { received = query; return { view: "compute_pool", machines: [] }; },
    close: () => new Promise(resolve => { release = resolve; }),
    free: () => { freed += 1; },
  };
  web.sessions.set("session", { fleet: domain => { assert.equal(domain, "domain"); return native; } });
  const id = await web.fleetOpen("session", "domain");
  assert.equal(JSON.parse(await web.fleetList(id, "{}", "read")).view, "domain");
  assert.deepEqual(received, { capabilities: [], matchAllCapabilities: false });
  await web.fleetComputePool(id, JSON.stringify({ mode: "dedicated", capabilities: ["vendor/v7"], match_all_capabilities: true }), "pool");
  assert.deepEqual(received, { mode: "dedicated", capabilities: ["vendor/v7"], matchAllCapabilities: true });
  await web.fleetOperationCancel("before-start");
  await assert.rejects(web.fleetList(id, "{}", "before-start"), error => error.kind === "cancelled");
  assert.equal(web.fleetOperations.size, 0);
  assert.equal(web.fleetCancelledBeforeStart.size, 0);
  const closing = web.fleetClose(id);
  assert.equal(freed, 0);
  release();
  await closing;
  await web.fleetClose(id);
  assert.equal(freed, 1);
  assert.equal(web.sessions.size, 1);
  await assert.rejects(web.fleetList(id, "{}", "after-close"), error => error.code === "closed");
  console.log("PASS Expo Web fleet adapter tests");
}
main().then(webAdapter).catch(error => { console.error(error); process.exitCode = 1; });
