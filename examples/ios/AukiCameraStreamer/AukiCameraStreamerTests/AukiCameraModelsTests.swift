import XCTest
@testable import AukiCameraStreamer

final class AukiCameraModelsTests: XCTestCase {
    func testDescriptorUsesPeerAndSessionStableIds() {
        let descriptor = AukiCameraDefaults.descriptor(peerId: "peer-a", sessionId: "session-b")
        XCTAssertEqual(descriptor.sensorId, "peer-a/session-b/camera")
        XCTAssertEqual(descriptor.frameId, "peer-a/session-b/camera_optical")
    }
}
