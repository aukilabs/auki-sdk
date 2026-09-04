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
  }
}
