// Served only by the Playwright test route. Never imported by the application/build.
export const AukiDiscoveryMode = { DiscoverAndAdvertise: 1 };
export const AukiPeerReachabilityMode = { RelayBacked: 1 };
// Keep spatial math real even when network/session state is mocked.
import initGeometry from "/pkg-web/auki_collaborative_mapping_web.js?geometry";
export { transformPoint, convertConventionPoint } from "/pkg-web/auki_collaborative_mapping_web.js?geometry";
export default async function init() { await initGeometry(); }
export class AukiUserSession {
  static async loginDev() {
    return new this();
  }
  async accessibleDomains() {
    return [{ id: "test-domain", name: "Test Domain", free() {} }];
  }
  async startPeerWithDiscovery() {
    return new AukiPeer();
  }
  async close() {}
  free() {}
}
export class AukiPeer {
  peerId = "local-test-peer";
  async discoverProtocol() {
    return [];
  }
  waitStopped() {
    return new Promise((r) => (this.stopped = r));
  }
  async shutdown() {
    this.stopped?.();
  }
  free() {}
}
export class AukiMapping {
  static async mount(_peer, _session, convention) {
    const fixture = new AukiMapping();
    fixture.local.convention = convention;
    return (window.fixture = fixture);
  }
  local = {
    peer: "local-test-peer",
    convention: "x_right",
    frame: "local-frame",
    sequence: 0,
    state: "aligned",
    portals: [],
    aligned_portals: [],
    to_display: {
      from_frame_id: "local-frame",
      to_frame_id: "local-frame",
      translation: [0, 0, 0],
      rotation_wxyz: [1, 0, 0, 0],
    },
  };
  data = {
    local_peer: "local-test-peer",
    display_frame: "local-frame",
    layers: [this.local],
    portals: [],
    conflicts: [],
  };
  view() {
    return JSON.stringify(this.data);
  }
  place(name, x, y, frame) {
    if (frame !== "local-frame") throw Error("Invalid frame");
    this.local.portals = this.local.portals.filter((p) => p.name !== name);
    this.local.portals.push({
      id: name,
      name,
      x,
      y,
      contributors: [this.local.peer],
    });
    this.local.aligned_portals = this.local.portals;
    this.local.sequence++;
    return this.view();
  }
  remove(name) {
    this.local.portals = this.local.portals.filter((p) => p.name !== name);
    this.local.aligned_portals = this.local.portals;
    this.local.sequence++;
    return this.view();
  }
  async close() {}
  free() {}
  async unfollow() {}
}
