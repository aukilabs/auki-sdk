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
