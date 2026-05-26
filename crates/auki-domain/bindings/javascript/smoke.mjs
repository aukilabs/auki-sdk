import { readFile } from "node:fs/promises";
import initAukiDomain, {
  clusterMembershipAdmitMemberJson,
  clusterMembershipFilenameJson,
  clusterMembershipNewJson,
  clusterMembershipPeerCountJson,
  domainSuccessorJson,
  electSuccessorJson,
  validateMembershipJson,
} from "./index.js";

const wasmBytes = await readFile(new URL("./$generated_wasm_file", import.meta.url));
await initAukiDomain({ module_or_path: wasmBytes });

function assert(condition, message) {
  if (!condition) {
    throw new Error(message);
  }
}

const peerA = "12D3KooWAvnEo4RaYZtqt2w83qzmQ7WVW2HhN2cay95EXAiVKcar";
const peerB = "12D3KooWJ9vbKFRy2E8XcY3wsXjghWcdq7tCEx86HGMGLevrnhmG";

let membership = clusterMembershipNewJson("demo");
assert(clusterMembershipFilenameJson(membership) === "demo.json", "filename vector failed");
membership = clusterMembershipAdmitMemberJson(
  membership,
  JSON.stringify({
    peer_id: peerA,
    multiaddrs: ["/ip4/127.0.0.1/tcp/4001"],
    join_ts_ns: 10,
  }),
);
membership = clusterMembershipAdmitMemberJson(
  membership,
  JSON.stringify({
    peer_id: peerB,
    multiaddrs: ["/ip4/127.0.0.1/tcp/4002"],
    join_ts_ns: 20,
  }),
);

assert(clusterMembershipPeerCountJson(membership) === 2n, "peer count vector failed");
assert(
  electSuccessorJson(membership, peerB, [peerA]) === peerA,
  "successor election vector failed",
);
assert(JSON.parse(validateMembershipJson(membership)).peers.length === 2, "membership validation failed");
assert(JSON.parse(domainSuccessorJson(membership, peerA)) === peerA, "domain successor vector failed");

console.log("javascript wasm smoke ok");
