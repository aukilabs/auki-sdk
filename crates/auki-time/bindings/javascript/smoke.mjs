import { readFile } from "node:fs/promises";
import init, {
  ClockSyncState,
  clockTransformEstimateTimeTransformJson,
  computeNtpSampleJson,
  defaultClockSyncConfigJson,
  estimateDomainClockJson,
  selectBestNtpSampleJson,
  timeTransformConvertNsJson,
} from "./$generated_js_file";

const wasmBytes = await readFile(new URL("./$generated_wasm_file", import.meta.url));
await init({ module_or_path: wasmBytes });

function assert(condition, message) {
  if (!condition) {
    throw new Error(message);
  }
}

const sample = JSON.parse(
  computeNtpSampleJson(
    JSON.stringify({
      local_send_ns: 1000,
      remote_receive_ns: 1001050,
      remote_send_ns: 1001080,
      local_receive_ns: 1130,
    }),
  ),
);
assert(sample.offset_ns === 1000000, "NTP offset vector failed");
assert(sample.uncertainty_ns === 100, "NTP uncertainty vector failed");

const best = JSON.parse(
  selectBestNtpSampleJson(
    JSON.stringify([
      { ...sample, uncertainty_ns: 500, observed_at_clock_ns: 2000 },
      { ...sample, uncertainty_ns: 50, observed_at_clock_ns: 3000 },
    ]),
  ),
);
assert(best.uncertainty_ns === 50, "best sample vector failed");

const state = new ClockSyncState(defaultClockSyncConfigJson());
const estimate = JSON.parse(
  state.observe(
    JSON.stringify({
      local_clock_id: "peer-a/session-1/monotonic",
      local_clock_hash: "hash-a",
      remote_clock_id: "peer-b/session-7/monotonic",
      remote_clock_hash: "hash-b",
      sample,
    }),
  ),
);
assert(estimate.offset_ns === 1000000, "clock sync state vector failed");
assert(JSON.parse(state.estimates()).length === 1, "clock sync estimates vector failed");

const domain = JSON.parse(
  estimateDomainClockJson(
    JSON.stringify(estimate),
    JSON.stringify({
      cluster_name: "cluster-a",
      domain_clock_id: "cluster-a/domain-clock",
      domain_clock_hash: "domain-hash",
      backing_peer_id: "12D3PeerB",
      backing_clock_id: "peer-b/session-7/monotonic",
      backing_clock_hash: "hash-b",
      backing_to_domain_offset_ns: 250,
    }),
  ),
);
assert(domain.total_offset_ns === 1000250, "domain clock vector failed");

const transform = JSON.parse(clockTransformEstimateTimeTransformJson(JSON.stringify(estimate)));
const converted = JSON.parse(timeTransformConvertNsJson(JSON.stringify(transform), 1130n));
assert(converted.timestamp_ns === 1001130, "time transform vector failed");

console.log("javascript wasm smoke ok");
