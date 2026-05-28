import XCTest
import AukiProto
import auki_domain
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

private final class AsyncRecordingDomainPeer: CameraDomainStreamPeer, @unchecked Sendable {
    private let lock = NSLock()
    private var syncPushCount = 0
    private var asyncPushes: [DomainStreamEntry] = []

    var counts: (sync: Int, async: Int) {
        lock.lock()
        defer { lock.unlock() }
        return (syncPushCount, asyncPushes.count)
    }

    func pushedEntry() -> DomainStreamEntry? {
        lock.lock()
        defer { lock.unlock() }
        return asyncPushes.first
    }

    func pushStreamEntry(streamId: UInt64, entry: DomainStreamEntry) throws {
        lock.lock()
        syncPushCount += 1
        lock.unlock()
    }

    func pushStreamEntryAsync(streamId: UInt64, entry: DomainStreamEntry) async throws {
        lock.lock()
        asyncPushes.append(entry)
        lock.unlock()
    }

    func finishStream(streamId: UInt64) throws {}
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

    func testDomainCameraStreamSinkUsesAsyncPushPath() async throws {
        let peer = AsyncRecordingDomainPeer()
        let sink = DomainCameraStreamSink(manager: peer)

        try await sink.pushCameraFrame(
            streamId: "42",
            timestampNs: 1_234,
            payload: Data([1, 2, 3])
        )

        let counts = peer.counts
        XCTAssertEqual(counts.sync, 0)
        XCTAssertEqual(counts.async, 1)
        let entry = try XCTUnwrap(peer.pushedEntry())
        XCTAssertEqual(entry.sequence, 0)
        XCTAssertEqual(entry.timestampNs, 1_234)
        XCTAssertEqual(entry.payload, Data([1, 2, 3]))
    }
}
