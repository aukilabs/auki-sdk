const assert = require("node:assert/strict");
const Module = require("node:module");
const ts = require("typescript");

require.extensions[".ts"] = (loaded, filename) => {
  const source = require("node:fs").readFileSync(filename, "utf8");
  loaded._compile(ts.transpileModule(source, {
    compilerOptions: {
      module: ts.ModuleKind.CommonJS,
      target: ts.ScriptTarget.ES2020,
      esModuleInterop: true,
    },
  }).outputText, filename);
};

const bridge = {};
const originalLoad = Module._load;
Module._load = function load(request, parent, isMain) {
  if (request === "expo") {
    return {
      NativeModule: class {},
      requireNativeModule: () => bridge,
    };
  }
  return originalLoad.call(this, request, parent, isMain);
};

const { data, domains } = require("../src/DomainData.ts");
const encoded = bytes => Buffer.from(bytes).toString("base64");
const metadata = {
  id: "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb",
  domain_id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
  name: "report",
  data_type: "app.report.v1",
  size: 2,
  created_at: "2026-09-01T00:00:00Z",
  updated_at: "2026-09-01T00:00:00Z",
};

async function main() {
  let capturedQuery;
  bridge.domainsList = async (session, query, operation) => {
    capturedQuery = { session, query: JSON.parse(query), operation };
    return { domains: [], total: 27, limit: 10, offset: 20 };
  };
  bridge.dataOperationCancel = async () => {};
  const page = await domains("session").list({ limit: 10, offset: 20 });
  assert.equal(page.total, 27);
  assert.deepEqual(capturedQuery.query, { limit: 10, offset: 20 });

  bridge.domainsDiscover = async (session, query, operation) => {
    assert.equal(session, "session");
    assert.deepEqual(JSON.parse(query), { allows: ["domain-data:r"], limit: 1, cursor: "next" });
    assert.ok(operation);
    return { domains: [], next_cursor: "sparse-next" };
  };
  const discovered = await domains("session").discover({ allows: ["domain-data:r"], limit: 1, cursor: "next" });
  assert.equal(discovered.next_cursor, "sparse-next");
  bridge.domainsForPortalPage = async (session, portal, organization, limit, cursor, operation) => {
    assert.deepEqual([session, portal, organization, limit, cursor], ["session", "ABC12345678", "own", 1, "next"]);
    assert.ok(operation);
    return { items: [], next_cursor: null, paginated: true };
  };
  assert.equal((await domains("session").forPortalPage("ABC12345678", 1, "next")).paginated, true);
  bridge.domainsPortalsPage = async (session, domain, limit, cursor, operation) => {
    assert.deepEqual([session, domain, limit, cursor], ["session", metadata.domain_id, 1, null]);
    assert.ok(operation);
    return { items: [], next_cursor: null, paginated: true };
  };
  assert.equal((await domains("session").portalsPage(metadata.domain_id, 1)).paginated, true);

  let cancelledOperation;
  let rejectDomainList;
  bridge.domainsList = (_session, _query, operation) => {
    cancelledOperation = operation;
    return new Promise((_resolve, reject) => { rejectDomainList = reject; });
  };
  bridge.dataOperationCancel = async operation => {
    assert.equal(operation, cancelledOperation);
    rejectDomainList(Object.assign(new Error("cancelled"), {
      code: "domain_data:cancelled::",
    }));
  };
  const listController = new AbortController();
  const cancelledList = domains("session").list({}, listController.signal);
  listController.abort();
  await assert.rejects(cancelledList, error => error.kind === "cancelled");
  bridge.dataOperationCancel = async () => {};

  bridge.domainDataOpen = async (session, domain) => {
    assert.equal(session, "session");
    assert.equal(domain, metadata.domain_id);
    return "client";
  };
  bridge.domainDataClose = async () => {};
  const client = await data("session", metadata.domain_id);

  const chunks = [encoded([1]), encoded([2]), null];
  let nextCalls = 0;
  let downloadClosed = 0;
  let downloadOptions;
  bridge.dataDownloadStart = async (_client, _id, options) => {
    downloadOptions = JSON.parse(options);
    return "download";
  };
  bridge.dataDownloadNext = async () => {
    nextCalls += 1;
    return chunks.shift();
  };
  bridge.dataDownloadCancel = async () => {};
  bridge.dataDownloadClose = async () => { downloadClosed += 1; };
  let releaseSink;
  const sinkGate = new Promise(resolve => { releaseSink = resolve; });
  const received = [];
  const reading = client.readTo(metadata.id, async bytes => {
    received.push(...bytes);
    if (received.length === 1) await sinkGate;
  }, { maxChunkBytes: 8 * 1024 * 1024 });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(nextCalls, 1, "download requested another chunk before the sink settled");
  releaseSink();
  assert.equal(await reading, 2);
  assert.deepEqual(received, [1, 2]);
  assert.equal(downloadOptions.maxChunkBytes, 256 * 1024);
  assert.equal(downloadClosed, 1);

  const maximums = [1024 * 1024, 1024 * 1024, 1, null];
  const supplied = [];
  bridge.dataUploadStart = async () => "upload";
  bridge.dataUploadNextMaximum = async () => maximums.shift();
  bridge.dataUploadPush = async (_id, value) => supplied.push(Buffer.from(value, "base64").length);
  bridge.dataUploadResult = async () => metadata;
  bridge.dataUploadCancel = async () => {};
  bridge.dataUploadClose = async () => {};
  const requested = [];
  assert.equal((await client.writeStream(
    { name: "report", dataType: "app.report.v1" },
    2,
    maximum => {
      requested.push(maximum);
      return requested.length <= 2 ? new Uint8Array([1]) : new Uint8Array();
    },
  )).id, metadata.id);
  assert.deepEqual(requested, [256 * 1024, 256 * 1024, 1]);
  assert.deepEqual(supplied, [1, 1, 0]);

  bridge.domainDataGet = async () => {
    throw Object.assign(new Error("permission denied"), { code: "domain_data:http:403:" });
  };
  await assert.rejects(client.get(metadata.id), error => error.kind === "http" && error.status === 403);
  bridge.domainDataGet = async () => {
    throw Object.assign(new Error("credential save failed"), {
      code: "domain_data:auth::persistence",
    });
  };
  await assert.rejects(client.get(metadata.id), error =>
    error.kind === "auth" && error.code === "persistence");

  let pendingDownloadReject;
  let transferCancelCalls = 0;
  bridge.dataDownloadStart = async () => "cancelled-download";
  bridge.dataDownloadNext = () => new Promise((_resolve, reject) => { pendingDownloadReject = reject; });
  bridge.dataDownloadCancel = async () => {
    transferCancelCalls += 1;
    pendingDownloadReject?.(Object.assign(new Error("cancelled"), {
      code: "domain_data:cancelled::",
    }));
  };
  bridge.dataDownloadClose = async () => {};
  const transferController = new AbortController();
  const cancelledRead = client.readTo(metadata.id, async () => {}, {}, transferController.signal);
  await new Promise(resolve => setTimeout(resolve, 0));
  transferController.abort();
  await assert.rejects(cancelledRead, error => error.kind === "cancelled");
  assert.equal(transferCancelCalls, 1);

  let uploadCancelCalls = 0;
  bridge.dataUploadStart = async () => "cancelled-upload";
  bridge.dataUploadNextMaximum = async () => 100;
  bridge.dataUploadCancel = async () => { uploadCancelCalls += 1; };
  bridge.dataUploadClose = async () => {};
  const sourceController = new AbortController();
  const cancelledWrite = client.writeStream(
    { name: "pending", dataType: "app.report.v1" },
    1,
    () => new Promise(() => {}),
    {},
    sourceController.signal,
  );
  await new Promise(resolve => setTimeout(resolve, 0));
  sourceController.abort();
  await assert.rejects(cancelledWrite, error => error.kind === "cancelled");
  assert.equal(uploadCancelCalls, 1, "pending source did not cancel its native upload");

  const denied = Object.assign(new Error("permission denied"), {
    code: "domain_data:http:403:",
  });
  bridge.dataDownloadStart = async () => "denied-download";
  bridge.dataDownloadNext = async () => { throw denied; };
  bridge.dataDownloadCancel = async () => {};
  bridge.dataDownloadClose = async () => { throw denied; };
  await assert.rejects(
    client.readTo(metadata.id, async () => {}),
    error => error.kind === "http" && error.status === 403,
  );

  const persistence = Object.assign(new Error("credential save failed"), {
    code: "domain_data:auth::persistence",
  });
  bridge.dataUploadStart = async () => "persistence-upload";
  bridge.dataUploadNextMaximum = async () => null;
  bridge.dataUploadResult = async () => { throw persistence; };
  bridge.dataUploadCancel = async () => {};
  bridge.dataUploadClose = async () => { throw persistence; };
  await assert.rejects(
    client.writeStream(
      { name: "retained", dataType: "app.report.v1" },
      1,
      () => new Uint8Array(),
    ),
    error => error.kind === "auth" && error.code === "persistence",
  );

  let cancelled = 0;
  let closedTransfer = 0;
  bridge.dataDownloadStart = async () => "failing-download";
  bridge.dataDownloadNext = async () => encoded([3]);
  bridge.dataDownloadCancel = async () => { cancelled += 1; };
  bridge.dataDownloadClose = async () => { closedTransfer += 1; };
  await assert.rejects(
    client.readTo(metadata.id, async () => { throw new Error("private destination failure"); }),
    error => error.kind === "callback" && !error.message.includes("private"),
  );
  assert.equal(cancelled, 1);
  assert.equal(closedTransfer, 1);

  bridge.dataDownloadStart = async () => "cleanup-download";
  bridge.dataDownloadNext = async () => encoded([4]);
  bridge.dataDownloadCancel = async () => {};
  bridge.dataDownloadClose = async () => {
    throw Object.assign(new Error("multipart cleanup failed"), {
      code: "domain_data:cleanup::",
    });
  };
  await assert.rejects(
    client.readTo(metadata.id, async () => { throw new Error("destination failed"); }),
    error => error.kind === "cleanup"
      && error.cause.operation.kind === "callback"
      && error.cause.cleanup.kind === "cleanup",
  );

  let releaseClose;
  const closeGate = new Promise(resolve => { releaseClose = resolve; });
  bridge.domainDataClose = async () => closeGate;
  let sinkEntered;
  const enteredSink = new Promise(resolve => { sinkEntered = resolve; });
  bridge.dataDownloadStart = async () => "close-download";
  bridge.dataDownloadNext = async () => encoded([5]);
  bridge.dataDownloadCancel = async () => {};
  bridge.dataDownloadClose = async () => {};
  const pendingRead = client.readTo(metadata.id, (_bytes, callbackSignal) => {
    sinkEntered();
    assert.equal(callbackSignal.aborted, false);
    return new Promise(() => {});
  });
  let sourceEntered;
  const enteredSource = new Promise(resolve => { sourceEntered = resolve; });
  let closeUploadCancelled = 0;
  bridge.dataUploadStart = async () => "close-upload";
  bridge.dataUploadNextMaximum = async () => 1;
  bridge.dataUploadCancel = async () => { closeUploadCancelled += 1; };
  bridge.dataUploadClose = async () => {};
  const pendingWrite = client.writeStream(
    { name: "close-pending", dataType: "app.report.v1" },
    1,
    (_maximum, callbackSignal) => {
      sourceEntered();
      assert.equal(callbackSignal.aborted, false);
      return new Promise(() => {});
    },
  );
  await enteredSink;
  await enteredSource;
  const pendingReadRejected = assert.rejects(pendingRead, error => error.kind === "cancelled");
  const pendingWriteRejected = assert.rejects(pendingWrite, error => error.kind === "cancelled");
  const closing = client.close();
  let closeSettled = false;
  void closing.then(() => { closeSettled = true; });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(closeSettled, false, "client close did not await native drain");
  await pendingReadRejected;
  await pendingWriteRejected;
  assert.equal(closeUploadCancelled, 1, "client close did not cancel a pending source");
  releaseClose();
  await closing;
  await assert.rejects(client.list(), error => error.kind === "closed");

  const controller = new AbortController();
  controller.abort();
  await assert.rejects(domains("session").list({}, controller.signal), error => error.kind === "cancelled");
  console.log("PASS Expo Domain data bridge tests");
}

main().catch(error => {
  console.error(error);
  process.exitCode = 1;
});
