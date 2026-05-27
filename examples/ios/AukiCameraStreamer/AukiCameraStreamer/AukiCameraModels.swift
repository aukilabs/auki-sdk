import Foundation

struct CapturedCameraFrame: Equatable {
    let jpegBytes: Data
    let timestampNs: UInt64
    let width: Int
    let height: Int
}

struct CameraSensorDescriptor: Equatable {
    let peerId: String
    let sessionId: String
    let sensorId: String
    let frameId: String
}

enum AukiCameraDefaults {
    static let sensorName = "ios-camera"
    static let segmentDurationNs: UInt64 = 1_000_000_000
    static let retentionNs: UInt64 = 300_000_000_000

    static func descriptor(peerId: String, sessionId: String) -> CameraSensorDescriptor {
        CameraSensorDescriptor(
            peerId: peerId,
            sessionId: sessionId,
            sensorId: "\(peerId)/\(sessionId)/camera",
            frameId: "\(peerId)/\(sessionId)/camera_optical"
        )
    }
}
