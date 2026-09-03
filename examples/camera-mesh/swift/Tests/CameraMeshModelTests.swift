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
}
