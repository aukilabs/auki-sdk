# Native iOS Producer Peer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a native iOS producer peer that imports the generated Swift bindings, joins or creates an Auki cluster, logs JPEG camera frames as typed `auki.camera.CameraFrame` entries, streams those entries through `DomainClusterManager`, and lets Overwatch join the same cluster and render the feed.

**Architecture:** Rust SDK crates stay authoritative for network, domain, registry, time, log, layout, and manifest behavior. The iOS app is a SwiftUI host over generated Swift packages, AVFoundation capture, SwiftProtobuf frame encoding, and small app-state orchestration. Overwatch remains the consumer UI, with a decode path for native `CameraFrame` stream payloads while keeping the current raw-JPEG demo path.

**Tech Stack:** Rust 2024, UniFFI 0.31, libp2p WebRTC Direct, `auki-network`, `auki-domain`, `auki-registry`, `auki-time`, `auki-logs`, `auki-layout`, `auki-manifests`, Swift 5.9, SwiftUI, AVFoundation, SwiftProtobuf, Xcode/iOS Simulator, TypeScript, Vite, Vitest, `@bufbuild/protobuf`.

---

## File Structure

- Modify: `crates/auki-domain/src/ffi.rs` - add a Swift-safe auto-advertise bootstrap helper for `DomainClusterManager`.
- Modify: `crates/auki-domain/src/readme.md` - document the helper and the native producer-peer bootstrap path.
- Modify: `crates/auki-domain/src/sprint.md` - record the iOS producer-peer binding milestone.
- Modify: `crates/auki-domain/changelog.md`, `crates/changelog.md`, `changelog.md` - propagate the domain binding change when it is implemented.
- Modify: `crates/auki-domain/tests/full_binding_surface.rs` - lock the new helper into the Rust binding-surface contract.
- Modify: `crates/auki-domain/bindings/swift/SmokeFullDomain/Sources/SmokeFullDomain/main.swift` - smoke the generated Swift helper with a loopback TCP listener.
- Modify: `examples/overwatch/scripts/stage-sdk.mjs` - stage generated `bindings/javascript/auki-proto` for Overwatch tests and development.
- Modify: `examples/overwatch/package.json` - add `@aukilabs/auki-proto` as a workspace-generated dependency.
- Modify: `examples/overwatch/src/sdk/streamHub.ts` - attach sensor kind to stream frames so preview code can choose protobuf decoding only for camera streams.
- Create: `examples/overwatch/src/data/cameraFramePayload.ts` - decode native `CameraFrame` bytes and preserve raw payload fallback.
- Modify: `examples/overwatch/src/data/preview.ts` - render decoded camera bytes when the frame is native camera protobuf.
- Modify: `examples/overwatch/src/data/preview.test.ts` - prove both native `CameraFrame` JPEG and raw JPEG payloads render.
- Modify: `examples/overwatch/src/sdk/streamHub.test.ts` - prove stream frames include the sensor kind when sensor metadata is present.
- Modify: `examples/overwatch/changelog.md`, `examples/changelog.md`, `changelog.md` - propagate Overwatch compatibility work when it is implemented.
- Create: `examples/ios/AukiCameraStreamer/README.md` - runbook for generating bindings, building the app, starting a cluster, and joining from Overwatch.
- Create: `examples/ios/AukiCameraStreamer/project.yml` - XcodeGen project that imports generated Swift packages.
- Create: `examples/ios/AukiCameraStreamer/AukiCameraStreamer/App.swift` - SwiftUI entry point.
- Create: `examples/ios/AukiCameraStreamer/AukiCameraStreamer/ContentView.swift` - operator controls and local preview.
- Create: `examples/ios/AukiCameraStreamer/AukiCameraStreamer/CameraStreamerViewModel.swift` - main state machine for permissions, cluster join, logging, and streaming.
- Create: `examples/ios/AukiCameraStreamer/AukiCameraStreamer/CameraCaptureService.swift` - AVFoundation capture service producing JPEG frames.
- Create: `examples/ios/AukiCameraStreamer/AukiCameraStreamer/AukiCameraSession.swift` - SDK-session coordinator for seed, clock, registry entries, sensor log, catalogs, and `DomainClusterManager`.
- Create: `examples/ios/AukiCameraStreamer/AukiCameraStreamer/CameraStreamFanout.swift` - tracks accepted stream ids and pushes encoded entries.
- Create: `examples/ios/AukiCameraStreamer/AukiCameraStreamer/AukiCameraModels.swift` - typed app DTOs and constants.
- Create: `examples/ios/AukiCameraStreamer/AukiCameraStreamer/KeychainSeedStore.swift` - persistent peer seed storage.
- Create: `examples/ios/AukiCameraStreamer/AukiCameraStreamerTests/CameraFrameEncodingTests.swift` - proves SwiftProtobuf output is an `auki.camera.CameraFrame` with JPEG bytes in field `frame`.
- Create: `examples/ios/AukiCameraStreamer/AukiCameraStreamerTests/CameraStreamFanoutTests.swift` - proves accepted streams receive pushed frame bytes and stopped streams are removed.
- Create: `examples/ios/AukiCameraStreamer/AukiCameraStreamerTests/AukiCameraModelsTests.swift` - proves ids and catalog JSON are stable.
- Modify: `examples/ios/changelog.md`, `examples/changelog.md`, `changelog.md` - propagate the new iOS example when it is implemented.
- Modify: `docs/superpowers/plans/2026-05-27-ios-camera-streamer.md` - mark tasks complete as implementation lands.
- Modify: `docs/superpowers/plans/changelog.md`, `docs/superpowers/changelog.md`, `docs/changelog.md`, `changelog.md` - propagate this plan and later status changes.

## Constraints

- Swift must not implement libp2p, Discovery, cluster admission, stream opening, registry hashing, manifest hashing, or log segmentation.
- The first iOS slice publishes JPEG camera frames only. ARKit pose, point clouds, audio, depth, and multi-camera selection are separate work.
- The iOS app uses `ClusterTargetMode.joinOrCreate`; the first reliable flow is iOS as native Manager/producer and Overwatch as browser consumer.
- Stream payloads from the native producer are encoded `auki.camera.CameraFrame` bytes. Overwatch keeps raw JPEG rendering for browser demos and old fixtures.
- The iOS app registers a camera sensor, frame, and session clock before it advertises a camera stream resource.
- The camera frame id is `<peer_id>/<session_id>/camera_optical` and uses the ROS optical convention created by `auki_registry.frameRosOpticalJson`.
- Sensor logs use `segment_duration_ns = 1_000_000_000` and `retention_ns = 300_000_000_000`.
- Device smoke testing requires camera permission and iOS local-network permission. Unit tests must run without camera hardware.

## Task 1: Add `DomainClusterManager` Auto-Advertise Bootstrap

**Files:**
- Modify: `crates/auki-domain/src/ffi.rs`
- Modify: `crates/auki-domain/tests/full_binding_surface.rs`
- Modify: `crates/auki-domain/bindings/swift/SmokeFullDomain/Sources/SmokeFullDomain/main.swift`
- Modify: `crates/auki-domain/src/readme.md`
- Modify: `crates/auki-domain/src/sprint.md`
- Modify: `crates/auki-domain/changelog.md`, `crates/changelog.md`, `changelog.md`

- [x] **Step 1: Add the failing Rust binding-surface test**

In `crates/auki-domain/tests/full_binding_surface.rs`, add a test next to `native_cluster_lifecycle_is_exposed` that calls the new helper with a loopback TCP listen address and no advertise override:

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_cluster_auto_advertise_lifecycle_is_exposed() {
    // binding-surface: native cluster lifecycle with auto-advertised addresses
    let server = MockDiscoveryServer::spawn(
        "binding-auto-advertise".to_string(),
        binding_peer_id(111),
    );
    let manager = auki_domain::bootstrap_domain_cluster_manager_auto_advertise(
        auki_domain::ClusterTargetMode::Create,
        "binding-auto-advertise".into(),
        vec![111; 32],
        vec!["/ip4/127.0.0.1/tcp/0".into()],
        vec![],
        1_000,
        server.base_url(),
        auki_domain::DaemonInfo {
            app: "binding-test".into(),
            name: "peer-111".into(),
            session_id: "session-111".into(),
            session_clock_id: "legacy-clock".into(),
            session_clock_hash: "legacy-clock-hash".into(),
            app_instance: "00163eabcdef".into(),
        },
        "auki-domain-binding-test/auto-advertise".into(),
    )
    .await
    .unwrap();

    assert_eq!(manager.cluster_name(), "binding-auto-advertise");
    assert!(manager.is_manager());
    assert!(!manager.local_peer_id().is_empty());
    manager.shutdown().await.unwrap();
}
```

Expected: `cargo test -p auki-domain --test full_binding_surface native_cluster_auto_advertise_lifecycle_is_exposed -- --nocapture` fails because the helper is not exported.

- [x] **Step 2: Implement the helper in `ffi.rs`**

Add this UniFFI export next to `bootstrap_domain_cluster_manager`:

```rust
#[uniffi::export(async_runtime = "tokio")]
pub async fn bootstrap_domain_cluster_manager_auto_advertise(
    target_mode: ClusterTargetMode,
    target_name: String,
    wallet_seed: Vec<u8>,
    listen_addrs: Vec<String>,
    advertise_multiaddrs_override: Vec<String>,
    advertise_resolution_ms: u64,
    discovery_url: String,
    daemon_info: core::DaemonInfo,
    agent_version: String,
) -> Result<Arc<DomainClusterManager>, BindingDomainError> {
    use std::time::Duration;

    let wallet = Wallet::from_seed(&seed32(wallet_seed)?);
    let identity = PeerIdentity::from_wallet(&wallet);
    let parsed_listen = parse_multiaddrs(listen_addrs)?;
    let mut swarm = build_swarm(
        &identity,
        SwarmConfig {
            listen_addresses: parsed_listen,
            agent_version,
            enable_relay_server: false,
        },
    )
    .map_err(|err| BindingDomainError::Network {
        message: err.to_string(),
    })?;

    let override_addrs = parse_multiaddrs(advertise_multiaddrs_override)?;
    let override_addrs = if override_addrs.is_empty() {
        None
    } else {
        Some(override_addrs.as_slice())
    };
    let local_multiaddrs = auki_network::swarm::resolve_advertise_multiaddrs(
        &mut swarm,
        override_addrs,
        Duration::from_millis(advertise_resolution_ms),
    )
    .await;

    bootstrap_domain_cluster_manager_with_swarm(
        target_mode,
        target_name,
        identity,
        local_multiaddrs,
        discovery_url,
        swarm,
        daemon_info,
    )
    .await
}
```

Extract the shared tail of the current `bootstrap_domain_cluster_manager` into a private `bootstrap_domain_cluster_manager_with_swarm(...)` helper so both bootstrap functions use the same `ClusterTargetMode` dispatch and stream decline policy.

- [x] **Step 3: Reuse existing multiaddr parsing**

Keep `parse_multiaddrs(...)` as the only string-to-`Multiaddr` parser. The new helper must not parse listen or advertise addresses in Swift.

- [x] **Step 4: Extend the Swift smoke**

In `crates/auki-domain/bindings/swift/SmokeFullDomain/Sources/SmokeFullDomain/main.swift`, after the current explicit-advertise manager shuts down, create a second manager:

```swift
let autoSeed = Data(repeating: 52, count: 32)
let autoClusterName = "swift-smoke-auto"
let autoServer = try MockDiscoveryServer(
    clusterName: autoClusterName,
    managerPeerId: try peerIdFromWalletSeed(seed: autoSeed)
)
let autoManager = try await bootstrapDomainClusterManagerAutoAdvertise(
    targetMode: .create,
    targetName: autoClusterName,
    walletSeed: autoSeed,
    listenAddrs: ["/ip4/127.0.0.1/tcp/0"],
    advertiseMultiaddrsOverride: [],
    advertiseResolutionMs: 1_000,
    discoveryUrl: autoServer.baseUrl,
    daemonInfo: DaemonInfo(
        app: "swift-smoke",
        name: "peer-52",
        sessionId: "session-52",
        sessionClockId: "legacy-clock-auto",
        sessionClockHash: "legacy-clock-auto-hash",
        appInstance: "00163eabcdf0"
    ),
    agentVersion: "auki-domain-swift-smoke/0.1"
)
precondition(autoManager.isManager(), "auto-advertise manager was not the creator")
try await autoManager.shutdown()
autoServer.stop()
```

- [x] **Step 5: Run the domain checks**

Run:

```bash
cargo test -p auki-domain --test full_binding_surface native_cluster_auto_advertise_lifecycle_is_exposed -- --nocapture
cargo test -p auki-domain --test full_binding_surface native_cluster_lifecycle_is_exposed native_byte_streams_are_exposed -- --nocapture
python3 scripts/bindings/generate_bindings.py generate swift auki-domain
swift run --package-path crates/auki-domain/bindings/swift/SmokeFullDomain
```

Expected: every command exits with status `0`.

- [x] **Step 6: Update docs and changelogs**

Update `crates/auki-domain/src/readme.md` and `crates/auki-domain/src/sprint.md` to state that generated Swift hosts can bootstrap a `DomainClusterManager` with SDK-resolved advertised listen addresses. Add leaf and parent changelog entries for the crate change.

- [x] **Step 7: Commit**

Run:

```bash
git add crates/auki-domain/src/ffi.rs crates/auki-domain/tests/full_binding_surface.rs crates/auki-domain/bindings/swift/SmokeFullDomain/Sources/SmokeFullDomain/main.swift crates/auki-domain/src/readme.md crates/auki-domain/src/sprint.md crates/auki-domain/changelog.md crates/changelog.md changelog.md
git commit -m "Expose domain auto-advertise bootstrap to Swift"
```

## Task 2: Teach Overwatch To Decode Native Camera Frames

**Files:**
- Modify: `examples/overwatch/scripts/stage-sdk.mjs`
- Modify: `examples/overwatch/package.json`
- Modify: `examples/overwatch/src/sdk/streamHub.ts`
- Create: `examples/overwatch/src/data/cameraFramePayload.ts`
- Modify: `examples/overwatch/src/data/preview.ts`
- Modify: `examples/overwatch/src/data/preview.test.ts`
- Modify: `examples/overwatch/src/sdk/streamHub.test.ts`
- Modify: `examples/overwatch/changelog.md`, `examples/changelog.md`, `changelog.md`

- [x] **Step 1: Add a failing native-camera preview test**

In `examples/overwatch/src/data/preview.test.ts`, add:

```ts
it("renders native camera frame protobuf payloads", async () => {
  const objectUrls: string[] = [];
  vi.stubGlobal("URL", {
    createObjectURL(blob: Blob) {
      objectUrls.push(`blob:${blob.size}:${blob.type}`);
      return objectUrls[objectUrls.length - 1];
    },
    revokeObjectURL: vi.fn(),
  });

  const jpeg = new Uint8Array([0xff, 0xd8, 0xff, 0xd9]);
  const cameraFrame = new Uint8Array([0x12, 0x04, ...jpeg]);
  const preview = await createPreview({
    payload: cameraFrame,
    sensorKind: "camera",
    timestampNs: 7n,
    sequence: 1n,
  });

  expect(preview.kind).toBe("image");
  expect(preview.url).toBe("blob:4:image/jpeg");
});
```

Expected: `npm --prefix examples/overwatch test -- preview.test.ts` fails because preview treats the protobuf envelope as JPEG bytes.

- [x] **Step 2: Stage `auki-proto` for Overwatch**

In `examples/overwatch/scripts/stage-sdk.mjs`, add `auki-proto` to the generated packages staged from `bindings/javascript`:

```js
const packages = [
  "auki-network",
  "auki-domain",
  "auki-geometry",
  "auki-proto",
];
```

In `examples/overwatch/package.json`, add:

```json
"@aukilabs/auki-proto": "file:.sdk/auki-proto"
```

Run:

```bash
scripts/generate-javascript-proto.sh
npm --prefix examples/overwatch install
```

- [x] **Step 3: Add the payload decoder**

Create `examples/overwatch/src/data/cameraFramePayload.ts`:

```ts
import { fromBinary } from "@bufbuild/protobuf";
import { CameraFrameSchema } from "@aukilabs/auki-proto/src/auki/camera_pb.js";

export function previewPayloadBytes(payload: Uint8Array, sensorKind?: string): Uint8Array {
  if (sensorKind !== "camera") {
    return payload;
  }

  const frame = fromBinary(CameraFrameSchema, payload);
  if (frame.frame.length === 0) {
    throw new Error("CameraFrame contained an empty frame field");
  }
  return frame.frame;
}
```

- [x] **Step 4: Use sensor kind in previews**

Update `examples/overwatch/src/data/preview.ts` so JPEG `Blob` creation uses:

```ts
const bytes = previewPayloadBytes(frame.payload, frame.sensorKind);
const blob = new Blob([bytes], { type: "image/jpeg" });
```

Keep the existing raw path by passing any non-camera or missing `sensorKind` through unchanged.

- [x] **Step 5: Attach sensor kind to stream frames**

Update `examples/overwatch/src/sdk/streamHub.ts` so `RuntimeStreamFrame` includes:

```ts
sensorKind?: SensorSummary["kind"];
```

When the stream subscription starts, find the matching sensor summary from the producer participant:

```ts
const sensor = sdkRuntime
  .getParticipantSensors(spec.peer_id)
  .find((candidate) => candidate.sensor_id === spec.sensor_id);
```

Set `sensorKind: sensor?.kind` on every emitted frame. Do not use string matching on sensor ids.

- [x] **Step 6: Prove stream metadata propagation**

In `examples/overwatch/src/sdk/streamHub.test.ts`, add a test where the fake runtime exposes a camera sensor and emits one `Entry` message. Assert the frame includes:

```ts
expect(frame.sensorKind).toBe("camera");
```

- [x] **Step 7: Run Overwatch checks**

Run:

```bash
npm --prefix examples/overwatch test -- preview.test.ts streamHub.test.ts
npm --prefix examples/overwatch run build
```

Expected: both commands exit with status `0`.

- [x] **Step 8: Update changelogs and commit**

Run:

```bash
git add examples/overwatch/scripts/stage-sdk.mjs examples/overwatch/package.json examples/overwatch/package-lock.json examples/overwatch/src/sdk/streamHub.ts examples/overwatch/src/data/cameraFramePayload.ts examples/overwatch/src/data/preview.ts examples/overwatch/src/data/preview.test.ts examples/overwatch/src/sdk/streamHub.test.ts examples/overwatch/changelog.md examples/changelog.md changelog.md
git commit -m "Decode native camera stream frames in Overwatch"
```

## Task 3: Scaffold The Native iOS Producer App

**Files:**
- Create: `examples/ios/AukiCameraStreamer/README.md`
- Create: `examples/ios/AukiCameraStreamer/project.yml`
- Create: `examples/ios/AukiCameraStreamer/AukiCameraStreamer/App.swift`
- Create: `examples/ios/AukiCameraStreamer/AukiCameraStreamer/ContentView.swift`
- Create: `examples/ios/AukiCameraStreamer/AukiCameraStreamer/AukiCameraModels.swift`
- Create: `examples/ios/AukiCameraStreamer/AukiCameraStreamerTests/AukiCameraModelsTests.swift`
- Modify: `examples/ios/changelog.md`, `examples/changelog.md`, `changelog.md`

- [x] **Step 1: Create the XcodeGen project**

Create `examples/ios/AukiCameraStreamer/project.yml`:

```yaml
name: AukiCameraStreamer
options:
  bundleIdPrefix: com.aukilabs.examples
  deploymentTarget:
    iOS: "17.0"
settings:
  base:
    SWIFT_VERSION: "5.9"
    IPHONEOS_DEPLOYMENT_TARGET: "17.0"
packages:
  auki-network:
    path: ../../../bindings/swift/auki-network
  auki-domain:
    path: ../../../bindings/swift/auki-domain
  auki-registry:
    path: ../../../bindings/swift/auki-registry
  auki-time:
    path: ../../../bindings/swift/auki-time
  auki-logs:
    path: ../../../bindings/swift/auki-logs
  auki-layout:
    path: ../../../bindings/swift/auki-layout
  auki-manifests:
    path: ../../../bindings/swift/auki-manifests
  auki-proto:
    path: ../../../bindings/swift/auki-proto
targets:
  AukiCameraStreamer:
    type: application
    platform: iOS
    sources:
      - AukiCameraStreamer
    dependencies:
      - package: auki-network
      - package: auki-domain
      - package: auki-registry
      - package: auki-time
      - package: auki-logs
      - package: auki-layout
      - package: auki-manifests
      - package: auki-proto
      - sdk: AVFoundation.framework
      - sdk: CoreImage.framework
      - sdk: ImageIO.framework
      - sdk: Security.framework
    settings:
      base:
        PRODUCT_BUNDLE_IDENTIFIER: com.aukilabs.examples.AukiCameraStreamer
        INFOPLIST_KEY_NSCameraUsageDescription: "Auki Camera Streamer captures camera frames to publish them into an Auki cluster."
        INFOPLIST_KEY_NSLocalNetworkUsageDescription: "Auki Camera Streamer uses the local network to connect to Auki cluster peers."
  AukiCameraStreamerTests:
    type: bundle.unit-test
    platform: iOS
    sources:
      - AukiCameraStreamerTests
    dependencies:
      - target: AukiCameraStreamer
      - package: auki-proto
```

- [x] **Step 2: Add minimal app entry**

Create `examples/ios/AukiCameraStreamer/AukiCameraStreamer/App.swift`:

```swift
import SwiftUI

@main
struct AukiCameraStreamerApp: App {
    var body: some Scene {
        WindowGroup {
            ContentView(viewModel: CameraStreamerViewModel())
        }
    }
}
```

- [x] **Step 3: Add stable models and constants**

Create `examples/ios/AukiCameraStreamer/AukiCameraStreamer/AukiCameraModels.swift`:

```swift
import Foundation

struct CapturedCameraFrame: Equatable {
    let jpegBytes: Data
    let timestampNs: UInt64
    let width: Int
    let height: Int
}

struct CameraSensorDescriptor: Equatable {
    let peerId: String
    let sessionId: String
    let sensorId: String
    let frameId: String
}

enum AukiCameraDefaults {
    static let sensorName = "ios-camera"
    static let segmentDurationNs: UInt64 = 1_000_000_000
    static let retentionNs: UInt64 = 300_000_000_000

    static func descriptor(peerId: String, sessionId: String) -> CameraSensorDescriptor {
        CameraSensorDescriptor(
            peerId: peerId,
            sessionId: sessionId,
            sensorId: "\(peerId)/\(sessionId)/camera",
            frameId: "\(peerId)/\(sessionId)/camera_optical"
        )
    }
}
```

- [x] **Step 4: Add a compact SwiftUI shell**

Create `examples/ios/AukiCameraStreamer/AukiCameraStreamer/ContentView.swift` with:

```swift
import SwiftUI

struct ContentView: View {
    @ObservedObject var viewModel: CameraStreamerViewModel

    var body: some View {
        NavigationStack {
            Form {
                Section("Cluster") {
                    TextField("Cluster name", text: $viewModel.clusterName)
                    TextField("Discovery URL", text: $viewModel.discoveryUrl)
                    Text(viewModel.peerId.isEmpty ? "Peer not started" : viewModel.peerId)
                        .font(.footnote.monospaced())
                }
                Section("Camera") {
                    Toggle("Logging enabled", isOn: $viewModel.loggingEnabled)
                    Toggle("Streaming enabled", isOn: $viewModel.streamingEnabled)
                    Text(viewModel.statusText)
                        .font(.footnote)
                }
                Section {
                    Button(viewModel.isRunning ? "Stop" : "Start") {
                        Task { await viewModel.toggleRunning() }
                    }
                }
            }
            .navigationTitle("Auki Camera")
        }
    }
}
```

The strings are operational UI labels only. Do not add instructional copy inside the app.

- [x] **Step 5: Add model tests**

Create `examples/ios/AukiCameraStreamer/AukiCameraStreamerTests/AukiCameraModelsTests.swift`:

```swift
import XCTest
@testable import AukiCameraStreamer

final class AukiCameraModelsTests: XCTestCase {
    func testDescriptorUsesPeerAndSessionStableIds() {
        let descriptor = AukiCameraDefaults.descriptor(peerId: "peer-a", sessionId: "session-b")
        XCTAssertEqual(descriptor.sensorId, "peer-a/session-b/camera")
        XCTAssertEqual(descriptor.frameId, "peer-a/session-b/camera_optical")
    }
}
```

- [x] **Step 6: Generate and build the project shell**

Run:

```bash
python3 scripts/bindings/generate_bindings.py generate swift auki-network
python3 scripts/bindings/generate_bindings.py generate swift auki-domain
python3 scripts/bindings/generate_bindings.py generate swift auki-registry
python3 scripts/bindings/generate_bindings.py generate swift auki-time
python3 scripts/bindings/generate_bindings.py generate swift auki-logs
python3 scripts/bindings/generate_bindings.py generate swift auki-layout
python3 scripts/bindings/generate_bindings.py generate swift auki-manifests
scripts/generate-swift-proto.sh
xcodegen generate --spec examples/ios/AukiCameraStreamer/project.yml
xcodebuild test -project examples/ios/AukiCameraStreamer/AukiCameraStreamer.xcodeproj -scheme AukiCameraStreamer -destination 'platform=iOS Simulator,name=iPhone 15'
```

Expected: the generated project builds and `AukiCameraModelsTests` passes.

- [x] **Step 7: Add README and changelogs, then commit**

Document the shell app, binding generation commands, and simulator test command in `examples/ios/AukiCameraStreamer/README.md`. Add leaf and parent changelog entries.

Run:

```bash
git add examples/ios/AukiCameraStreamer examples/ios/changelog.md examples/changelog.md changelog.md
git commit -m "Add iOS camera streamer app shell"
```

## Task 4: Add Camera Frame Encoding And Stream Fanout

**Files:**
- Create: `examples/ios/AukiCameraStreamer/AukiCameraStreamer/CameraStreamFanout.swift`
- Modify: `examples/ios/AukiCameraStreamer/AukiCameraStreamer/CameraStreamerViewModel.swift`
- Create: `examples/ios/AukiCameraStreamer/AukiCameraStreamerTests/CameraFrameEncodingTests.swift`
- Create: `examples/ios/AukiCameraStreamer/AukiCameraStreamerTests/CameraStreamFanoutTests.swift`

- [x] **Step 1: Add a failing SwiftProtobuf encoding test**

Create `examples/ios/AukiCameraStreamer/AukiCameraStreamerTests/CameraFrameEncodingTests.swift`:

```swift
import XCTest
import AukiProto
@testable import AukiCameraStreamer

final class CameraFrameEncodingTests: XCTestCase {
    func testCameraFrameEncodingPlacesJpegBytesInFrameField() throws {
        let jpeg = Data([0xff, 0xd8, 0xff, 0xd9])
        let encoded = try CameraFrameCodec.encode(jpegBytes: jpeg)
        let decoded = try Auki_Camera_CameraFrame(serializedBytes: encoded)
        XCTAssertEqual(decoded.frame, jpeg)
    }
}
```

Expected: the app tests fail because `CameraFrameCodec` does not exist.

- [x] **Step 2: Implement the codec**

Add to `CameraStreamFanout.swift`:

```swift
import Foundation
import AukiProto

enum CameraFrameCodec {
    static func encode(jpegBytes: Data) throws -> Data {
        var frame = Auki_Camera_CameraFrame()
        frame.frame = jpegBytes
        return try frame.serializedData()
    }
}
```

- [x] **Step 3: Add a sink protocol for fanout tests**

In `CameraStreamFanout.swift`, define:

```swift
protocol CameraStreamSink {
    func pushCameraFrame(streamId: String, timestampNs: UInt64, payload: Data) async throws
    func finishStream(streamId: String) async throws
}

actor CameraStreamFanout {
    private var activeStreamIds: Set<String> = []
    private let sink: CameraStreamSink

    init(sink: CameraStreamSink) {
        self.sink = sink
    }

    func accept(streamId: String) {
        activeStreamIds.insert(streamId)
    }

    func remove(streamId: String) {
        activeStreamIds.remove(streamId)
    }

    func streamCount() -> Int {
        activeStreamIds.count
    }

    func push(_ frame: CapturedCameraFrame) async throws {
        guard !activeStreamIds.isEmpty else {
            return
        }
        let payload = try CameraFrameCodec.encode(jpegBytes: frame.jpegBytes)
        for streamId in activeStreamIds {
            try await sink.pushCameraFrame(
                streamId: streamId,
                timestampNs: frame.timestampNs,
                payload: payload
            )
        }
    }
}
```

- [x] **Step 4: Prove fanout behavior**

Create `examples/ios/AukiCameraStreamer/AukiCameraStreamerTests/CameraStreamFanoutTests.swift`:

```swift
import XCTest
@testable import AukiCameraStreamer

private actor RecordingSink: CameraStreamSink {
    struct Push: Equatable {
        let streamId: String
        let timestampNs: UInt64
        let payload: Data
    }

    private(set) var pushes: [Push] = []

    func pushCameraFrame(streamId: String, timestampNs: UInt64, payload: Data) async throws {
        pushes.append(Push(streamId: streamId, timestampNs: timestampNs, payload: payload))
    }

    func finishStream(streamId: String) async throws {}
}

final class CameraStreamFanoutTests: XCTestCase {
    func testPushesEncodedFrameToAcceptedStreams() async throws {
        let sink = RecordingSink()
        let fanout = CameraStreamFanout(sink: sink)
        await fanout.accept(streamId: "stream-a")
        try await fanout.push(CapturedCameraFrame(
            jpegBytes: Data([0xff, 0xd8, 0xff, 0xd9]),
            timestampNs: 42,
            width: 1,
            height: 1
        ))

        let pushes = await sink.pushes
        XCTAssertEqual(pushes.count, 1)
        XCTAssertEqual(pushes[0].streamId, "stream-a")
        XCTAssertEqual(pushes[0].timestampNs, 42)
    }
}
```

- [x] **Step 5: Run app tests and commit**

Run:

```bash
xcodebuild test -project examples/ios/AukiCameraStreamer/AukiCameraStreamer.xcodeproj -scheme AukiCameraStreamer -destination 'platform=iOS Simulator,name=iPhone 15'
```

Expected: app tests pass.

Run:

```bash
git add examples/ios/AukiCameraStreamer
git commit -m "Encode and fan out iOS camera frames"
```

## Task 5: Implement Registry, Log, And Domain Session Coordination

**Files:**
- Create: `examples/ios/AukiCameraStreamer/AukiCameraStreamer/KeychainSeedStore.swift`
- Create: `examples/ios/AukiCameraStreamer/AukiCameraStreamer/AukiCameraSession.swift`
- Modify: `examples/ios/AukiCameraStreamer/AukiCameraStreamer/CameraStreamFanout.swift`
- Modify: `examples/ios/AukiCameraStreamer/AukiCameraStreamer/CameraStreamerViewModel.swift`
- Modify: `examples/ios/AukiCameraStreamer/AukiCameraStreamerTests/AukiCameraModelsTests.swift`
- Modify: `examples/ios/AukiCameraStreamer/README.md`
- Modify: `examples/ios/changelog.md`, `examples/changelog.md`, `changelog.md`

- [x] **Step 1: Add stable session catalog tests**

In `AukiCameraModelsTests.swift`, add tests that assert the generated sensor catalog and resource catalog JSON contain:

```swift
XCTAssertEqual(sensor["sensor_id"] as? String, "peer-a/session-b/camera")
XCTAssertEqual(sensor["kind"] as? String, "camera")
XCTAssertEqual(resource["kind"] as? String, "sensor_stream")
XCTAssertEqual(resource["sensor_id"] as? String, "peer-a/session-b/camera")
```

Expected: tests fail until the session builder exists.

- [x] **Step 2: Add seed persistence**

Create `KeychainSeedStore.swift` with:

```swift
import Foundation
import Security

final class KeychainSeedStore {
    private let account = "AukiCameraStreamer.walletSeed"

    func loadOrCreate() throws -> Data {
        if let existing = try load() {
            return existing
        }
        var seed = Data(count: 32)
        let result = seed.withUnsafeMutableBytes { pointer in
            SecRandomCopyBytes(kSecRandomDefault, 32, pointer.baseAddress!)
        }
        guard result == errSecSuccess else {
            throw NSError(domain: "AukiCameraStreamer.KeychainSeedStore", code: Int(result))
        }
        try save(seed)
        return seed
    }

    private func load() throws -> Data? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrAccount as String: account,
            kSecReturnData as String: true
        ]
        var item: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &item)
        if status == errSecItemNotFound {
            return nil
        }
        guard status == errSecSuccess else {
            throw NSError(domain: "AukiCameraStreamer.KeychainSeedStore", code: Int(status))
        }
        return item as? Data
    }

    private func save(_ seed: Data) throws {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrAccount as String: account,
            kSecValueData as String: seed
        ]
        SecItemDelete(query as CFDictionary)
        let status = SecItemAdd(query as CFDictionary, nil)
        guard status == errSecSuccess else {
            throw NSError(domain: "AukiCameraStreamer.KeychainSeedStore", code: Int(status))
        }
    }
}
```

- [x] **Step 3: Build `AukiCameraSession` over generated SDK packages**

Create `AukiCameraSession.swift` that imports:

```swift
import Foundation
import auki_domain
import auki_logs
import auki_manifests
import auki_registry
import auki_time
```

The session startup sequence must be:

1. Load or create the 32-byte wallet seed.
2. Create a `SessionClock` with `peerId`, `sessionId`, and `"ios-camera"`.
3. Build the camera frame entry with `frameRosOpticalJson(id:)`.
4. Build the sensor entry with `pixel_format = "jpeg"`, `color_space = "srgb"`, `intrinsics_model = "pinhole"`, and `distortion_model = "unknown"`.
5. Write clock, frame, and sensor entries through generated `auki_registry` helpers under the app root.
6. Build the sensor log manifest with `buildSensorLogManifestJson`.
7. Open a `BytesLog`, set retention to `300_000_000_000`, and append each encoded `CameraFrame`.
8. Bootstrap `DomainClusterManager` through `bootstrapDomainClusterManagerAutoAdvertise` with listen address `"/ip4/0.0.0.0/udp/0/webrtc-direct"`.
9. Register static sensor catalog, resource catalog, and registry entries on the manager.
10. Poll `drainStreamOpenRequests`, accept matching camera requests, and decline others with `sensor_not_found` or `sensor_unavailable`.

- [x] **Step 4: Connect fanout to the generated domain manager**

In `CameraStreamFanout.swift`, add a concrete sink:

```swift
final class DomainCameraStreamSink: CameraStreamSink {
    private let manager: DomainClusterManager

    init(manager: DomainClusterManager) {
        self.manager = manager
    }

    func pushCameraFrame(streamId: String, timestampNs: UInt64, payload: Data) async throws {
        try manager.pushStreamEntry(
            streamId: streamId,
            timestampNs: timestampNs,
            payloadKind: "camera",
            payload: payload
        )
    }

    func finishStream(streamId: String) async throws {
        try manager.finishStream(streamId: streamId)
    }
}
```

Use the exact generated Swift method labels after regeneration if UniFFI lowercases or renames the Rust arguments.

- [x] **Step 5: Add logging path**

When a captured frame arrives and logging is enabled:

```swift
let payload = try CameraFrameCodec.encode(jpegBytes: frame.jpegBytes)
try sensorLog.append(timestampNs: frame.timestampNs, payload: payload)
try await fanout.push(frame)
```

The log payload and stream payload must be the same encoded `CameraFrame` bytes.

- [x] **Step 6: Run checks and commit**

Run:

```bash
xcodebuild test -project examples/ios/AukiCameraStreamer/AukiCameraStreamer.xcodeproj -scheme AukiCameraStreamer -destination 'platform=iOS Simulator,name=iPhone 15'
git diff --check
```

Expected: both commands exit with status `0`.

Run:

```bash
git add examples/ios/AukiCameraStreamer examples/ios/changelog.md examples/changelog.md changelog.md
git commit -m "Connect iOS camera streamer to domain sessions"
```

## Task 6: Capture Camera Frames With AVFoundation

**Files:**
- Create: `examples/ios/AukiCameraStreamer/AukiCameraStreamer/CameraCaptureService.swift`
- Modify: `examples/ios/AukiCameraStreamer/AukiCameraStreamer/CameraStreamerViewModel.swift`
- Modify: `examples/ios/AukiCameraStreamer/AukiCameraStreamer/ContentView.swift`
- Modify: `examples/ios/AukiCameraStreamer/README.md`

- [x] **Step 1: Add the capture protocol**

Create a testable abstraction:

```swift
protocol CameraCaptureServiceDelegate: AnyObject {
    func cameraCaptureService(_ service: CameraCaptureService, didCapture frame: CapturedCameraFrame)
    func cameraCaptureService(_ service: CameraCaptureService, didFail error: Error)
}

protocol CameraCaptureControlling {
    func requestAccess() async -> Bool
    func start() async throws
    func stop() async
}
```

- [x] **Step 2: Implement AVFoundation capture**

`CameraCaptureService` must use:

- `AVCaptureSession`
- `AVCaptureDevice.default(.builtInWideAngleCamera, for: .video, position: .back)`
- `AVCaptureVideoDataOutput`
- `kCVPixelFormatType_32BGRA`
- A serial `DispatchQueue(label: "AukiCameraStreamer.capture")`
- `CIContext.jpegRepresentation(of:colorSpace:options:)` for JPEG bytes

Throttle capture to a conservative default of 10 frames per second:

```swift
private let minimumFrameIntervalNs: UInt64 = 100_000_000
```

Use `SessionClock.nowNs()` from `AukiCameraSession` for frame timestamps once the SDK session is running. Before the SDK session starts, do not emit frames to the log or stream.

- [x] **Step 3: Wire preview state**

Store the latest local JPEG as a `UIImage` in `CameraStreamerViewModel` for a local preview. Keep preview rendering out of the SDK path; the SDK path uses encoded bytes from `CapturedCameraFrame`.

- [ ] **Step 4: Build on simulator and device**

Run the simulator build:

```bash
xcodebuild build -project examples/ios/AukiCameraStreamer/AukiCameraStreamer.xcodeproj -scheme AukiCameraStreamer -destination 'platform=iOS Simulator,name=iPhone 15'
```

On a physical iPhone, run from Xcode and verify:

- Camera permission prompt appears.
- Local-network permission prompt appears when cluster bootstrap starts.
- The local preview updates.
- The status line reports active logging.

Status: Simulator build and test coverage passed on `iPhone 15,OS=17.5`. Physical iPhone verification remains pending under Task 8's physical-device smoke.

- [x] **Step 5: Commit**

Run:

```bash
git add examples/ios/AukiCameraStreamer
git commit -m "Capture iOS camera frames for streaming"
```

## Task 7: Complete View Model State And Manual E2E Runbook

**Files:**
- Modify: `examples/ios/AukiCameraStreamer/AukiCameraStreamer/CameraStreamerViewModel.swift`
- Modify: `examples/ios/AukiCameraStreamer/AukiCameraStreamer/ContentView.swift`
- Modify: `examples/ios/AukiCameraStreamer/README.md`
- Modify: `examples/ios/changelog.md`, `examples/changelog.md`, `changelog.md`

- [x] **Step 1: Implement the view model state machine**

`CameraStreamerViewModel` must expose:

```swift
@Published var clusterName: String = "ios-camera"
@Published var discoveryUrl: String = "http://192.168.9.130:8080"
@Published var loggingEnabled: Bool = true
@Published var streamingEnabled: Bool = true
@Published private(set) var peerId: String = ""
@Published private(set) var isRunning: Bool = false
@Published private(set) var statusText: String = "Stopped"
@Published private(set) var lastPreviewImage: UIImage?
```

`toggleRunning()` must:

1. Request camera access.
2. Start `AukiCameraSession`.
3. Start camera capture.
4. Stop capture, finish streams, flush the log, and shut down the manager when stopping.

- [x] **Step 2: Add operator status without instructional copy**

Show only live values:

- Peer id
- Session id
- Accepted stream count
- Logged frame count
- Last frame timestamp
- Last error

- [x] **Step 3: Document E2E runbook**

In `examples/ios/AukiCameraStreamer/README.md`, document:

```bash
python3 scripts/bindings/generate_bindings.py generate swift auki-domain
python3 scripts/bindings/generate_bindings.py generate swift auki-network
python3 scripts/bindings/generate_bindings.py generate swift auki-registry
python3 scripts/bindings/generate_bindings.py generate swift auki-time
python3 scripts/bindings/generate_bindings.py generate swift auki-logs
python3 scripts/bindings/generate_bindings.py generate swift auki-layout
python3 scripts/bindings/generate_bindings.py generate swift auki-manifests
scripts/generate-swift-proto.sh
xcodegen generate --spec examples/ios/AukiCameraStreamer/project.yml
npm --prefix examples/overwatch install
npm --prefix examples/overwatch run dev
```

Manual flow:

1. Launch iOS app on device.
2. Set the same Discovery URL Overwatch uses.
3. Start the app with cluster name `ios-camera`.
4. Open Overwatch.
5. Join cluster `ios-camera`.
6. Select the iOS camera sensor.
7. Confirm the preview updates from the iOS stream.

- [x] **Step 4: Commit**

Run:

```bash
git add examples/ios/AukiCameraStreamer examples/ios/changelog.md examples/changelog.md changelog.md
git commit -m "Finish iOS camera streamer controls"
```

## Task 8: Final Verification And Plan Closeout

**Files:**
- Modify: `docs/superpowers/plans/2026-05-27-ios-camera-streamer.md`
- Modify: `docs/superpowers/plans/changelog.md`
- Modify: `docs/superpowers/changelog.md`
- Modify: `docs/changelog.md`
- Modify: `changelog.md`

- [x] **Step 1: Run full relevant automated checks**

Run:

```bash
cargo test -p auki-domain --test full_binding_surface
python3 scripts/bindings/generate_bindings.py generate swift auki-domain
swift run --package-path crates/auki-domain/bindings/swift/SmokeFullDomain
scripts/generate-javascript-proto.sh
npm --prefix examples/overwatch test -- preview.test.ts streamHub.test.ts
npm --prefix examples/overwatch run build
xcodebuild test -project examples/ios/AukiCameraStreamer/AukiCameraStreamer.xcodeproj -scheme AukiCameraStreamer -destination 'platform=iOS Simulator,name=iPhone 15'
git diff --check
```

Expected: every command exits with status `0`.

Status on May 27, 2026 HKT:

- `cargo test -p auki-domain --test full_binding_surface` passed: 14 tests, 0 failures.
- `python3 scripts/bindings/generate_bindings.py generate swift auki-domain` exited `0`.
- `swift run --package-path crates/auki-domain/bindings/swift/SmokeFullDomain` exited `0`.
- `scripts/generate-javascript-proto.sh` exited `0`.
- `npm --prefix examples/overwatch test -- preview.test.ts streamHub.test.ts` passed: 3 files, 7 tests.
- `npm --prefix examples/overwatch run build` exited `0`.
- `xcodebuild test ... -destination 'platform=iOS Simulator,name=iPhone 15'` did not run because Xcode resolved it to `OS:latest` and this machine has `iPhone 15` only on iOS 17.5.
- `xcodebuild test ... -destination 'platform=iOS Simulator,name=iPhone 15,OS=17.5'` passed: 8 tests, 0 failures.
- `git diff --check` exited `0`.

- [ ] **Step 2: Run physical-device smoke**

On an iPhone and a desktop running Overwatch:

- iOS app starts cluster `ios-camera` with WebRTC Direct.
- Overwatch joins the same cluster.
- Overwatch lists the iOS camera sensor.
- Overwatch opens the camera stream.
- iOS accepts the stream request.
- Overwatch preview displays the iOS camera frames.
- iOS sensor log receives encoded `CameraFrame` entries.

Record the device model, iOS version, Discovery URL, Overwatch URL, and cluster name in the implementation closeout.

Status: Not run in this Codex session. This requires a physical iPhone run, camera and local-network permission prompts, and a desktop Overwatch session on the same Discovery URL.

- [ ] **Step 3: Mark this plan complete**

Check off every completed task in this file. Add changelog entries under `docs/superpowers/plans/changelog.md`, `docs/superpowers/changelog.md`, `docs/changelog.md`, and root `changelog.md` describing completion.

- [ ] **Step 4: Commit closeout**

Run:

```bash
git add docs/superpowers/plans/2026-05-27-ios-camera-streamer.md docs/superpowers/plans/changelog.md docs/superpowers/changelog.md docs/changelog.md changelog.md
git commit -m "Complete iOS camera streamer plan"
```

## Self-Review Checklist

- [x] The app is a native iOS producer peer, not a Swift libp2p implementation.
- [x] The cluster path uses generated `auki-domain` Swift bindings and `DomainClusterManager`.
- [x] The iOS app logs and streams the same encoded `CameraFrame` bytes.
- [x] Overwatch decodes native camera stream payloads and preserves the raw JPEG demo path.
- [x] Registry entries include camera sensor, ROS optical frame, and session clock.
- [x] Stream-open requests accept only the advertised camera sensor.
- [x] Automated checks cover Rust binding surface, generated Swift smoke, Overwatch preview decoding, and iOS unit tests.
- [ ] Physical-device smoke verifies camera permission, local-network permission, cluster join, stream acceptance, and Overwatch preview rendering.
