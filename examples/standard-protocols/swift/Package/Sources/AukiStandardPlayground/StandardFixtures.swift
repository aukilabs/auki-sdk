import AukiSDK
import Foundation

public enum StandardFixtures {
  public static let application = "standard-protocols"
  public static let applicationVersion = "0.1.0"
  public static let sessionID = "playground-session"
  public static let messageResourceID = "playground/events"
  public static let messageClockID = "playground/clock"
  public static let messageClockHash = "playground-clock-v1"
  public static let messageType = "playground.message"
  public static let messageTimestampNs: Int64 = 42
  public static let messageBytes = Data("hello from the standard protocol playground".utf8)
  public static let streamResourceID = "playground/scalar"
  public static let streamTimestampNs: Int64 = 99
  public static let streamValue = 12.5
  public static let registryID = "playground/base"
  public static let blobBytes = Data("auki-standard-protocols-v1".utf8)
  public static let blobSHA256 = "bc170af4cf7bb5266683f459f5121348f60a7a5ee7d35a9bf7f5d29fe8fa3b96"

  public static let protocolIDs = [
    "/auki/auth/1/info/1.0.0",
    "/auki/auth/1/resources/0.3.0",
    "/auki/auth/1/resources/0.4.0",
    "/auki/auth/1/registries/0.3.0",
    "/auki/auth/1/blobs/0.1.0",
    "/auki/auth/1/message/0.1.0",
    "/auki/auth/1/stream/0.2.0",
  ]

  public static func info(peerID: String, nodeName: String) -> ParticipantInfo {
    ParticipantInfo(
      app: application,
      appVersion: applicationVersion,
      name: nodeName,
      sessionID: sessionID,
      sessionClockID: messageClockID,
      sessionClockHash: messageClockHash,
      sessionNowNs: 0,
      peerID: peerID,
      appInstance: "swift"
    )
  }

  public static func messageChannel(peerID: String) -> AukiMessageChannel {
    AukiMessageChannel(
      ownerPeerId: peerID,
      resourceId: messageResourceID,
      clock: AukiMessageClockReference(
        peerId: peerID,
        id: messageClockID,
        hash: messageClockHash
      )
    )
  }

  public static func catalogResources(peerID: String) -> CatalogResourcesSnapshot {
    CatalogResourcesSnapshot(resources: [
      CatalogMessageChannel(
        variant: "message_channel",
        ownerPeerID: peerID,
        resourceID: messageResourceID,
        clock: CatalogClockReference(
          peerID: peerID,
          id: messageClockID,
          hash: messageClockHash
        )
      )
    ])
  }

  public static func frameRegistry(peerID: String) -> FrameRegistryFixture {
    FrameRegistryFixture(
      axes: ["x": "forward", "y": "left", "z": "up"],
      frameID: registryID,
      handedness: "right",
      peerID: peerID,
      units: "meters"
    )
  }

  public static func streamManifest() throws -> AukiStreamManifest {
    try streamManifestFromJson(
      json: json(
        StreamManifestFixture(
          resourceID: streamResourceID,
          payload: "scalar"
        )))
  }

  public static func scalarBytes(_ value: Double = streamValue) -> Data {
    var bits = value.bitPattern.littleEndian
    var bytes = Data([0x09])
    Swift.withUnsafeBytes(of: &bits) { bytes.append(contentsOf: $0) }
    return bytes
  }

  public static func scalarValue(from bytes: Data) throws -> Double {
    guard bytes.count == 9, bytes.first == 0x09 else {
      throw StandardFixtureError("Scalar protobuf must be one fixed64 field")
    }
    var bits: UInt64 = 0
    for (index, byte) in bytes.dropFirst().enumerated() {
      bits |= UInt64(byte) << UInt64(index * 8)
    }
    return Double(bitPattern: bits)
  }

  public static func json<T: Encodable>(_ value: T) throws -> String {
    let encoder = JSONEncoder()
    encoder.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
    return String(decoding: try encoder.encode(value), as: UTF8.self)
  }

  public static func decode<T: Decodable>(_ type: T.Type, from json: String) throws -> T {
    try JSONDecoder().decode(type, from: Data(json.utf8))
  }

  public static func resourcesAreEmpty(json: String) throws -> Bool {
    guard
      let object = try JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any],
      let resources = object["resources"] as? [Any]
    else {
      throw StandardFixtureError("Catalog response is missing its resources array")
    }
    return resources.isEmpty
  }

  public static func isRegistryHash(_ value: String) -> Bool {
    value.utf8.count == 32
      && value.utf8.allSatisfy {
        (48...57).contains($0) || (97...102).contains($0)
      }
  }
}

public struct ParticipantInfo: Codable, Equatable, Sendable {
  public let app: String
  public let appVersion: String
  public let name: String
  public let sessionID: String
  public let sessionClockID: String
  public let sessionClockHash: String
  public let sessionNowNs: Int64
  public let peerID: String
  public let appInstance: String

  enum CodingKeys: String, CodingKey {
    case app
    case appVersion = "app_version"
    case name
    case sessionID = "session_id"
    case sessionClockID = "session_clock_id"
    case sessionClockHash = "session_clock_hash"
    case sessionNowNs = "session_now_ns"
    case peerID = "peer_id"
    case appInstance = "app_instance"
  }
}

public struct CatalogResourcesSnapshot: Codable, Equatable, Sendable {
  public let resources: [CatalogMessageChannel]
}

public struct CatalogMessageChannel: Codable, Equatable, Sendable {
  public let variant: String
  public let ownerPeerID: String
  public let resourceID: String
  public let clock: CatalogClockReference

  enum CodingKeys: String, CodingKey {
    case variant
    case ownerPeerID = "owner_peer_id"
    case resourceID = "resource_id"
    case clock
  }
}

public struct CatalogClockReference: Codable, Equatable, Sendable {
  public let peerID: String
  public let id: String
  public let hash: String

  enum CodingKeys: String, CodingKey {
    case peerID = "peer_id"
    case id
    case hash
  }
}

public struct FrameRegistryFixture: Codable, Equatable, Sendable {
  public let axes: [String: String]
  public let frameID: String
  public let handedness: String
  public let peerID: String
  public let units: String

  enum CodingKeys: String, CodingKey {
    case axes
    case frameID = "frame_id"
    case handedness
    case peerID = "peer_id"
    case units
  }
}

private struct StreamManifestFixture: Codable {
  let resourceID: String
  let payload: String

  enum CodingKeys: String, CodingKey {
    case resourceID = "resourceId"
    case payload
  }
}

public struct StandardFixtureError: LocalizedError, Sendable {
  public let message: String

  public init(_ message: String) {
    self.message = message
  }

  public var errorDescription: String? { message }
}
