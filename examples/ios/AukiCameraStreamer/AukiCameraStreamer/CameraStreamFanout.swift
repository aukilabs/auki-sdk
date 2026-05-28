import Foundation
import AukiProto
import AukiDomainSignaledWebRTC
import auki_domain

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
        try await pushEncodedPayload(timestampNs: frame.timestampNs, payload: payload)
    }

    func pushEncodedPayload(timestampNs: UInt64, payload: Data) async throws {
        guard !activeStreamIds.isEmpty else {
            return
        }

        let streamIds = activeStreamIds.sorted()
        var failedStreamIds: [String] = []
        var firstError: Error?

        for streamId in streamIds {
            do {
                try await sink.pushCameraFrame(
                    streamId: streamId,
                    timestampNs: timestampNs,
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

    func finishAll() async throws {
        let streamIds = activeStreamIds.sorted()
        var finishedStreamIds: [String] = []
        var firstError: Error?

        for streamId in streamIds {
            do {
                try await sink.finishStream(streamId: streamId)
                finishedStreamIds.append(streamId)
            } catch {
                if firstError == nil {
                    firstError = error
                }
            }
        }

        activeStreamIds.subtract(finishedStreamIds)

        if let firstError {
            throw firstError
        }
    }
}

enum DomainCameraStreamSinkError: Error {
    case invalidStreamId(String)
}

protocol CameraDomainStreamPeer: AnyObject, Sendable {
    func pushStreamEntry(streamId: UInt64, entry: DomainStreamEntry) throws
    func pushStreamEntryAsync(streamId: UInt64, entry: DomainStreamEntry) async throws
    func finishStream(streamId: UInt64) throws
}

extension DomainClusterManager {
    func pushStreamEntryAsync(streamId: UInt64, entry: DomainStreamEntry) async throws {
        try pushStreamEntry(streamId: streamId, entry: entry)
    }
}

extension DomainClusterManager: CameraDomainStreamPeer {}
extension AukiSignaledWebRTCDomainPeer: CameraDomainStreamPeer {}

actor DomainCameraStreamSink: CameraStreamSink {
    private let manager: CameraDomainStreamPeer
    private var nextSequenceByStreamId: [String: UInt64] = [:]

    init(manager: CameraDomainStreamPeer) {
        self.manager = manager
    }

    func pushCameraFrame(streamId: String, timestampNs: UInt64, payload: Data) async throws {
        guard let numericStreamId = UInt64(streamId) else {
            throw DomainCameraStreamSinkError.invalidStreamId(streamId)
        }

        let sequence = nextSequenceByStreamId[streamId, default: 0]
        nextSequenceByStreamId[streamId] = sequence + 1

        try await manager.pushStreamEntryAsync(
            streamId: numericStreamId,
            entry: DomainStreamEntry(
                sequence: sequence,
                timestampNs: timestampNs,
                payloadKind: "camera",
                payload: payload
            )
        )
    }

    func finishStream(streamId: String) async throws {
        defer {
            nextSequenceByStreamId.removeValue(forKey: streamId)
        }
        guard let numericStreamId = UInt64(streamId) else {
            throw DomainCameraStreamSinkError.invalidStreamId(streamId)
        }

        try manager.finishStream(streamId: numericStreamId)
    }
}
