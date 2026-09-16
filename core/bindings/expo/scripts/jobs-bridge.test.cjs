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

main().catch(error => { console.error(error); process.exitCode = 1; });
