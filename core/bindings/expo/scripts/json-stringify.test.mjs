import assert from "node:assert/strict";
import { jsonStringify } from "../build/json-stringify.js";

const catalog = {
  resources: [
    {
      variant: "sensor_log",
      resource_id: "camera/main",
      state: "live",
      sensor: { kind: "camera" },
      available: {
        bytes: 3_000_000_000n,
        duration_ns: 5_000_000_000n,
        entries: 900n,
      },
      head: { kind: "rolling", retention_ns: 5_000_000_000n },
    },
  ],
};

assert.throws(() => JSON.stringify(catalog), /BigInt/);

const json = jsonStringify(catalog);
const parsed = JSON.parse(json);
assert.equal(parsed.resources[0].resource_id, "camera/main");
assert.equal(parsed.resources[0].available.duration_ns, "5000000000");
assert.equal(parsed.resources[0].head.retention_ns, "5000000000");

const info = jsonStringify({ sessionNowNs: 1_700_000_000_000_000_000n, app: "booster_k1_runner" });
assert.equal(JSON.parse(info).sessionNowNs, "1700000000000000000");

const pretty = jsonStringify({ sessionNowNs: 1n }, 2);
assert.match(pretty, /\n/);
assert.equal(JSON.parse(pretty).sessionNowNs, "1");

console.log("json-stringify: ok");
