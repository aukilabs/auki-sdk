import XCTest
import AukiProto
@testable import AukiCameraStreamer

private enum RecordingSinkError: Error {
    case rejected
}

private actor RecordingSink: CameraStreamSink {
    struct Push: Equatable {
        let streamId: String
        let timestampNs: UInt64
        let payload: Data
    }

    private(set) var pushes: [Push] = []
    private let failingStreamIds: Set<String>

    init(failingStreamIds: Set<String> = []) {
        self.failingStreamIds = failingStreamIds
    }

    func pushCameraFrame(streamId: String, timestampNs: UInt64, payload: Data) async throws {
        if failingStreamIds.contains(streamId) {
            throw RecordingSinkError.rejected
        }

        pushes.append(Push(streamId: streamId, timestampNs: timestampNs, payload: payload))
    }

    func finishStream(streamId: String) async throws {}
}

final class CameraStreamFanoutTests: XCTestCase {
    func testPushesEncodedFrameToAcceptedStreams() async throws {
        let sink = RecordingSink()
        let fanout = CameraStreamFanout(sink: sink)
        let jpeg = Data([0xff, 0xd8, 0xff, 0xd9])

        await fanout.accept(streamId: "stream-a")
        try await fanout.push(CapturedCameraFrame(
            jpegBytes: jpeg,
            timestampNs: 42,
            width: 1,
            height: 1
        ))

        let pushes = await sink.pushes
        XCTAssertEqual(pushes.count, 1)
        XCTAssertEqual(pushes[0].streamId, "stream-a")
        XCTAssertEqual(pushes[0].timestampNs, 42)

        let decoded = try Auki_Camera_CameraFrame(serializedBytes: pushes[0].payload)
        XCTAssertEqual(decoded.frame, jpeg)
    }

    func testRemovePreventsFuturePushes() async throws {
        let sink = RecordingSink()
        let fanout = CameraStreamFanout(sink: sink)

        await fanout.accept(streamId: "stream-a")
        await fanout.remove(streamId: "stream-a")
        try await fanout.push(CapturedCameraFrame(
            jpegBytes: Data([0xff, 0xd8, 0xff, 0xd9]),
            timestampNs: 43,
            width: 1,
            height: 1
        ))

        let pushes = await sink.pushes
        XCTAssertTrue(pushes.isEmpty)
        let count = await fanout.streamCount()
        XCTAssertEqual(count, 0)
    }

    func testFailingStreamDoesNotBlockHealthyStreamsAndIsRemoved() async throws {
        let sink = RecordingSink(failingStreamIds: ["stream-bad"])
        let fanout = CameraStreamFanout(sink: sink)

        await fanout.accept(streamId: "stream-good-a")
        await fanout.accept(streamId: "stream-bad")
        await fanout.accept(streamId: "stream-good-b")

        do {
            try await fanout.push(CapturedCameraFrame(
                jpegBytes: Data([0xff, 0xd8, 0xff, 0xd9]),
                timestampNs: 44,
                width: 1,
                height: 1
            ))
            XCTFail("Expected failing stream to report an error")
        } catch RecordingSinkError.rejected {}

        let pushedStreamIds = await Set(sink.pushes.map(\.streamId))
        XCTAssertEqual(pushedStreamIds, ["stream-good-a", "stream-good-b"])
        let count = await fanout.streamCount()
        XCTAssertEqual(count, 2)
    }
}
