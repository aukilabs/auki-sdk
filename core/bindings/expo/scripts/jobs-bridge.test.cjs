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

const { jobs } = require("../src/Jobs.ts");
const domainId = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const jobId = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
const spec = {
  label: "third-party job",
  tasks: [{
    label: "vendor",
    stage: "analyze",
    capability: "vendor.example/private-model/v7",
    mode: "dedicated",
    capabilityFilters: { model: "custom" },
  }],
};

async function main() {
  bridge.jobsOpen = async () => {
    throw Object.assign(new Error("session closed"), { code: "closed" });
  };
  await assert.rejects(jobs("closed-session", domainId), error => error.kind === "closed");

  bridge.jobsOpen = async (session, domain) => {
    assert.equal(session, "session"); assert.equal(domain, domainId); return "jobs-client";
  };
  bridge.jobsOperationCancel = async () => {};
  bridge.jobsClose = async () => {};
  const client = await jobs("session", domainId);

  let submitted = 0;
  bridge.jobsEstimate = async (_client, payload) => {
    const decoded = JSON.parse(payload);
    assert.equal(decoded.tasks[0].capability, "vendor.example/private-model/v7");
    assert.equal(decoded.tasks[0].capability_filters.model, "custom");
    return JSON.stringify({ total: "12.500", tasks: [] });
  };
  bridge.jobsSubmit = async (_client, payload) => {
    submitted += 1;
    assert.equal(JSON.parse(payload).tasks[0].capability_filters.model, "custom");
    return jobId;
  };
  assert.equal((await client.estimate(spec)).total, "12.500");
  assert.equal(await client.submit(spec), jobId);
  assert.equal(submitted, 1);

  let keyedCalls = 0;
  bridge.jobsSubmitWithKey = async (id, payload, key, operation) => {
    keyedCalls += 1;
    assert.equal(id, "jobs-client");
    assert.equal(key, "persisted-key");
    assert.notEqual(operation, key);
    assert.equal(JSON.parse(payload).tasks[0].capability_filters.model, "custom");
    if (keyedCalls === 1) throw Object.assign(new Error("busy"), { code: "jobs:submission_in_progress:409:::1" });
    if (keyedCalls === 2) throw Object.assign(new Error("uncertain"), { code: "jobs:submission_uncertain:503::http_status:" });
    return jobId;
  };
  await assert.rejects(client.submitWithKey(spec, "persisted-key"), error =>
    error.kind === "submission_in_progress" && error.status === 409 && error.retryAfterSeconds === 1);
  assert.equal(keyedCalls, 1, "busy submission was automatically retried");
  await assert.rejects(client.submitWithKey(spec, "persisted-key"), error =>
    error.kind === "submission_uncertain" && error.status === 503);
  assert.equal(keyedCalls, 2, "uncertain keyed submission was automatically retried");
  assert.equal(await client.submitWithKey(spec, "persisted-key"), jobId);
  const preCancelled = new AbortController();
  preCancelled.abort();
  await assert.rejects(client.submitWithKey(spec, "persisted-key", preCancelled.signal), error => error.kind === "cancelled");
  assert.equal(keyedCalls, 3, "pre-cancelled submission reached the bridge");

  bridge.jobsList = async (_client, query) => {
    assert.deepEqual(JSON.parse(query), { limit: 10, capabilities: ["vendor.example/private-model/v7"], match_all_capabilities: true });
    return JSON.stringify({ items: [], next_cursor: "opaque" });
  };
  assert.equal((await client.list({ limit: 10, capabilities: [spec.tasks[0].capability], matchAllCapabilities: true })).next_cursor, "opaque");

  bridge.jobsGet = async (_client, id) => {
    assert.equal(id, jobId);
    return JSON.stringify({ job: { id }, tasks_summary: {}, tasks: [], receipts: [] });
  };
  assert.equal((await client.get(jobId)).job.id, jobId);

  bridge.jobsCancel = async () => JSON.stringify({ id: jobId, status: "canceled", updated_at: "2026-09-16T00:00:00Z" });
  assert.equal((await client.cancel(jobId)).status, "canceled");

  bridge.jobsSubmit = async () => {
    throw Object.assign(new Error("submission outcome is unknown"), {
      code: "jobs:submission_uncertain:::transport",
    });
  };
  await assert.rejects(client.submit(spec), error => error.kind === "submission_uncertain" && error.source === "transport");
  assert.equal(submitted, 1, "ambiguous submission was retried");

  bridge.jobsGet = async () => {
    throw Object.assign(new Error("bad response"), { kind: "response" });
  };
  await assert.rejects(client.get(jobId), error => error.kind === "invalid_response");

  const circular = { run: "local" };
  circular.self = circular;
  const invalidSpec = { ...spec, meta: circular };
  const inputController = new AbortController();
  let listeners = 0;
  const add = inputController.signal.addEventListener.bind(inputController.signal);
  const remove = inputController.signal.removeEventListener.bind(inputController.signal);
  inputController.signal.addEventListener = (...args) => { listeners += 1; return add(...args); };
  inputController.signal.removeEventListener = (...args) => { listeners -= 1; return remove(...args); };
  await assert.rejects(client.estimate(invalidSpec, inputController.signal), error => error.kind === "invalid_input");
  assert.equal(listeners, 0, "synchronous input failure retained its abort listener");

  bridge.jobsList = async () => {
    throw Object.assign(new Error("credential save failed"), { code: "jobs:auth::persistence:" });
  };
  await assert.rejects(client.list(), error => error.kind === "auth" && error.code === "persistence");

  let cancelledOperation;
  let rejectEstimate;
  const cancelledBeforeStart = new Set();
  bridge.jobsEstimate = (_client, _payload, operation) => {
    cancelledOperation = operation;
    if (cancelledBeforeStart.delete(operation)) {
      return Promise.reject(Object.assign(new Error("cancelled"), { code: "jobs:cancelled:::" }));
    }
    return new Promise((_resolve, reject) => { rejectEstimate = reject; });
  };
  bridge.jobsOperationCancel = async operation => {
    if (rejectEstimate) {
      assert.equal(operation, cancelledOperation);
      rejectEstimate(Object.assign(new Error("cancelled"), { code: "jobs:cancelled:::" }));
    } else {
      cancelledBeforeStart.add(operation);
    }
  };
  const controller = new AbortController();
  const pending = client.estimate(spec, controller.signal);
  controller.abort();
  await assert.rejects(pending, error => error.kind === "cancelled");

  let releaseClose;
  bridge.jobsClose = () => new Promise(resolve => { releaseClose = resolve; });
  const closing = client.close();
  await assert.rejects(client.list(), error => error.kind === "closed");
  let settled = false;
  void closing.then(() => { settled = true; });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(settled, false, "jobs close did not await native cleanup");
  releaseClose();
  await closing;
  await client.close();
  console.log("PASS Expo jobs bridge tests");
}

async function webAdapter() {
  const previousLoad = Module._load;
  Module._load = function load(request, parent, isMain) {
    if (request === "expo") return { NativeModule: class {}, registerWebModule: type => new type() };
    if (request === "./web/loadAukiSdkWasm") return { loadAukiSdkWasm: async () => { throw Error("unexpected peer/runtime initialization"); } };
    return previousLoad.call(this, request, parent, isMain);
  };
  const web = require("../src/AukiSdkExpoModule.web.ts").default;
  let calls = 0;
  let freed = false;
  const busy = Object.assign(new Error("busy"), { kind: "submission_in_progress", code: "submission_in_progress", status: 409, retryAfterSeconds: 1 });
  const native = {
    submitWithKey: async (request, key, signal) => {
      assert.equal(key, "persisted-key");
      assert.equal(request.tasks[0].capabilityFilters.model, "custom");
      assert.equal(request.idempotency_key, undefined);
      if (signal.aborted) throw Object.assign(new Error("cancelled"), { kind: "cancelled" });
      if (++calls === 1) throw busy;
      return jobId;
    },
    close: async () => {},
    free: () => { freed = true; },
  };
  web.sessions.set("session", { jobs: domain => { assert.equal(domain, domainId); return native; } });
  const id = await web.jobsOpen("session", domainId);
  const payload = JSON.stringify({label: "job", tasks: [{label: "task", stage: "task", capability: "vendor/v7", capability_filters: {model: "custom"}}]});
  await assert.rejects(web.jobsSubmitWithKey(id, payload, "persisted-key", "first"), error => error === busy);
  assert.equal(calls, 1);
  assert.equal(await web.jobsSubmitWithKey(id, payload, "persisted-key", "retry"), jobId);
  await web.jobsOperationCancel("cancelled");
  await assert.rejects(web.jobsSubmitWithKey(id, payload, "persisted-key", "cancelled"), error => error.kind === "cancelled");
  assert.equal(calls, 2);
  assert.equal(web.jobsOperations.size, 0);
  assert.equal(web.jobsCancelledBeforeStart.size, 0);
  await web.jobsClose(id);
  assert.equal(freed, true);
  console.log("PASS Expo Web keyed jobs adapter tests");
}
main().then(webAdapter).catch(error => { console.error(error); process.exitCode = 1; });
