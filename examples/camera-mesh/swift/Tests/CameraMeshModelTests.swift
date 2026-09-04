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
}
