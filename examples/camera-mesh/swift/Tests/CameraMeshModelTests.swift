import XCTest

@testable import AukiCameraMeshIOS

final class CameraMeshModelTests: XCTestCase {
  func testRolesChooseExplicitDiscoveryBehavior() {
    XCTAssertEqual(CameraMeshRole.viewer.discoveryMode, .discoverOnly)
    XCTAssertEqual(CameraMeshRole.publisher.discoveryMode, .discoverAndAdvertise)
  }

  func testApprovalRequiredDetectionMatchesPortableErrors() {
    XCTAssertTrue(isCameraApprovalRequired("approval_required: camera is hidden"))
    XCTAssertTrue(isCameraApprovalRequired("Approval required before viewing"))
    XCTAssertFalse(isCameraApprovalRequired("access_denied"))
  }

  func testPhoneCameraWallUsesOneOrTwoColumns() {
    XCTAssertEqual(effectiveCameraColumnCount(requested: 1, compact: true), 1)
    XCTAssertEqual(effectiveCameraColumnCount(requested: 2, compact: true), 2)
    XCTAssertEqual(effectiveCameraColumnCount(requested: 3, compact: true), 2)
    XCTAssertEqual(effectiveCameraColumnCount(requested: 4, compact: true), 2)
  }

  func testWideCameraWallClampsColumnSelection() {
    XCTAssertEqual(effectiveCameraColumnCount(requested: 0, compact: false), 1)
    XCTAssertEqual(effectiveCameraColumnCount(requested: 3, compact: false), 3)
    XCTAssertEqual(effectiveCameraColumnCount(requested: 9, compact: false), 4)
  }

  func testCameraWallDensityChoosesNewConnectionQuality() {
    XCTAssertEqual(preferredCameraQuality(forColumnCount: 1), .high)
    XCTAssertEqual(preferredCameraQuality(forColumnCount: 2), .medium)
    XCTAssertEqual(preferredCameraQuality(forColumnCount: 3), .low)
    XCTAssertEqual(preferredCameraQuality(forColumnCount: 4), .low)
  }

  @MainActor
  func testCameraTileReportsReceiveRenderBandwidthAndAge() {
    let tile = CameraTile(
      peerID: "camera-peer",
      status: .live,
      message: "Live feed"
    )

    for index in 0...4 {
      let time = Double(index) * 0.2
      tile.recordReceivedFrame(bytes: 1_024, at: time)
      if index.isMultiple(of: 2) {
        tile.recordRenderedFrame(
          frameCount: UInt64(index + 1),
          timestampNs: 1_000_000_000,
          at: time,
          wallClock: 1.123
        )
      }
    }

    XCTAssertEqual(tile.diagnostics.receiveFPS ?? 0, 5, accuracy: 0.001)
    XCTAssertEqual(tile.diagnostics.renderFPS ?? 0, 2.5, accuracy: 0.001)
    XCTAssertEqual(tile.diagnostics.kibPerSecond ?? 0, 5, accuracy: 0.001)
    XCTAssertEqual(tile.diagnostics.frameAgeMilliseconds ?? 0, 123, accuracy: 0.001)
    XCTAssertEqual(tile.totalReceivedFrames, 5)
    XCTAssertEqual(tile.totalRenderedFrames, 3)
    XCTAssertEqual(tile.totalReceivedBytes, 5_120)
  }

  @MainActor
  func testCameraTileResetClearsRollingDiagnostics() {
    let tile = CameraTile(
      peerID: "camera-peer",
      status: .live,
      message: "Live feed"
    )
    tile.recordReceivedFrame(bytes: 1_024, at: 0)
    tile.recordReceivedFrame(bytes: 1_024, at: 0.2)
    tile.recordRenderedFrame(
      frameCount: 1,
      timestampNs: 1_000_000_000,
      at: 0.2,
      wallClock: 1.05
    )

    tile.resetDiagnostics()

    XCTAssertEqual(tile.diagnostics, .empty)
    XCTAssertEqual(tile.totalReceivedFrames, 2)
    XCTAssertEqual(tile.totalRenderedFrames, 1)
    XCTAssertEqual(tile.totalReceivedBytes, 2_048)
  }

  func testPerformanceReportUsesWindowCountersAndPortableJSONKeys() throws {
    let capture = CameraPerformanceCapture(
      context: CameraPerformanceContext(
        runtime: "ios",
        platform: "iPhone test",
        domainID: "domain-1",
        localPeerID: "viewer-1",
        columnCount: 2
      ),
      startedAt: Date(timeIntervalSince1970: 1_000),
      startedAtMonotonic: 10
    )

    capture.sample(
      [performanceSnapshot(received: 10, rendered: 5, bytes: 10_000, receiveFPS: 10)],
      columnCount: 2,
      nowMonotonic: 10
    )
    capture.recordEvent("Switched to High", nowMonotonic: 10.5)
    capture.sample(
      [performanceSnapshot(received: 15, rendered: 7, bytes: 16_000, receiveFPS: 12)],
      columnCount: 4,
      nowMonotonic: 11
    )
    let report = capture.finish(
      snapshots: [
        performanceSnapshot(received: 18, rendered: 9, bytes: 20_000, receiveFPS: 14)
      ],
      finalColumnCount: 4,
      endedAt: Date(timeIntervalSince1970: 1_002),
      endedAtMonotonic: 12
    )

    XCTAssertEqual(report.schemaVersion, 1)
    XCTAssertEqual(report.kind, cameraPerformanceReportKind)
    XCTAssertEqual(report.durationMs, 2_000)
    XCTAssertEqual(report.initialColumns, 2)
    XCTAssertEqual(report.finalColumns, 4)
    XCTAssertEqual(
      report.events, [CameraPerformanceEvent(elapsedMs: 500, message: "Switched to High")])
    XCTAssertEqual(report.peers.count, 1)
    XCTAssertEqual(report.peers[0].summary.receivedFrames, 8)
    XCTAssertEqual(report.peers[0].summary.renderedFrames, 4)
    XCTAssertEqual(report.peers[0].summary.receivedBytes, 10_000)
    XCTAssertEqual(report.peers[0].summary.renderToReceiveRatio, 0.5)
    XCTAssertEqual(report.peers[0].summary.receiveFps?.p95, 14)

    let json = try report.json()
    let decoded = try XCTUnwrap(
      JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any]
    )
    XCTAssertEqual(decoded["domainId"] as? String, "domain-1")
    XCTAssertEqual(decoded["localPeerId"] as? String, "viewer-1")
    XCTAssertNil(decoded["domainID"])
  }
}

private func performanceSnapshot(
  received: UInt64,
  rendered: UInt64,
  bytes: UInt64,
  receiveFPS: Double
) -> CameraPerformanceSnapshot {
  CameraPerformanceSnapshot(
    peerID: "camera-1",
    name: "Lab camera",
    runtime: "native",
    status: "live",
    quality: "high",
    width: 1_920,
    height: 1_080,
    targetFPS: 30,
    totalReceivedFrames: received,
    totalRenderedFrames: rendered,
    totalReceivedBytes: bytes,
    receiveFPS: receiveFPS,
    renderFPS: receiveFPS - 2,
    kibPerSecond: 1_000,
    frameAgeMilliseconds: 80
  )
}
