import Foundation
import AukiProto

enum CameraFrameCodec {
    static func encode(jpegBytes: Data) throws -> Data {
        var frame = Auki_Camera_CameraFrame()
        frame.frame = jpegBytes
        return try frame.serializedData()
    }
}

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
        let streamIds = activeStreamIds.sorted()
        var failedStreamIds: [String] = []
        var firstError: Error?

        for streamId in streamIds {
            do {
                try await sink.pushCameraFrame(
                    streamId: streamId,
                    timestampNs: frame.timestampNs,
                    payload: payload
                )
            } catch {
                failedStreamIds.append(streamId)
                if firstError == nil {
                    firstError = error
                }
            }
        }

        activeStreamIds.subtract(failedStreamIds)

        if let firstError {
            throw firstError
        }
    }
}
