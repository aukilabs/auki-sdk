import Foundation
import ImageIO

public enum CameraQuality: String, CaseIterable, Hashable, Identifiable, Sendable {
  case low
  case medium
  case high

  public var id: Self { self }

  public var title: String {
    rawValue.prefix(1).uppercased() + rawValue.dropFirst()
  }

  public var shortTitle: String {
    switch self {
    case .low: "L"
    case .medium: "M"
    case .high: "H"
    }
  }
}

public struct CameraStreamProfile: Equatable, Sendable {
  public let quality: CameraQuality
  public let resourceID: String
  public let width: Int
  public let height: Int
  public let rateHz: Int

  public init(
    quality: CameraQuality,
    resourceID: String,
    width: Int,
    height: Int,
    rateHz: Int
  ) {
    self.quality = quality
    self.resourceID = resourceID
    self.width = width
    self.height = height
    self.rateHz = rateHz
  }

  public var label: String {
    "\(quality.title) · \(width)×\(height) @ \(rateHz) fps"
  }
}

public enum CameraMeshContract {
  public static let application = "auki-camera-mesh"
  public static let applicationVersion = "0.1.0"
  public static let cameraResourceID = "camera/main"
  public static let controlResourceID = "camera/control"
  public static let replyResourceID = "camera/replies"
  public static let clockID = "camera/utc"
  public static let frameID = "camera/optical"
  public static let width = 480
  public static let height = 270
  public static let rateHz = 5
  public static let cameraMediumResourceID = "camera/main/medium"
  public static let cameraHighResourceID = "camera/main/high"
  public static let maximumViewerConnections = 16
  public static let maximumPendingSnapshots = 16
  public static let maximumBlobBytes = 20 * 1024 * 1024
  public static let snapshotTimeoutSeconds = 45

  public static let infoProtocolID = "/auki/auth/1/info/1.0.0"
  public static let catalogResourcesProtocolID = "/auki/auth/1/resources/0.3.0"
  public static let catalogMapsProtocolID = "/auki/auth/1/resources/0.4.0"
  public static let registryProtocolID = "/auki/auth/1/registries/0.3.0"
  public static let blobProtocolID = "/auki/auth/1/blobs/0.1.0"
  public static let messageProtocolID = "/auki/auth/1/message/0.1.0"
  public static let streamProtocolID = "/auki/auth/1/stream/0.2.0"

  public static let viewerProtocolIDs = [
    infoProtocolID,
    catalogResourcesProtocolID,
    catalogMapsProtocolID,
    registryProtocolID,
    blobProtocolID,
    messageProtocolID,
  ]

  public static let publisherProtocolIDs = viewerProtocolIDs + [streamProtocolID]

  public static let profiles: [CameraStreamProfile] = [
    CameraStreamProfile(
      quality: .low,
      resourceID: cameraResourceID,
      width: width,
      height: height,
      rateHz: rateHz
    ),
    CameraStreamProfile(
      quality: .medium,
      resourceID: cameraMediumResourceID,
      width: 960,
      height: 540,
      rateHz: 15
    ),
    CameraStreamProfile(
      quality: .high,
      resourceID: cameraHighResourceID,
      width: 1_920,
      height: 1_080,
      rateHz: 30
    ),
  ]

  public static func profile(_ quality: CameraQuality) -> CameraStreamProfile {
    profiles.first(where: { $0.quality == quality })!
  }

  /// Conservative batch size for the Camera Mesh demo's measured relay path.
  public static func addAllLimit(for quality: CameraQuality) -> Int {
    switch quality {
    case .low: 16
    case .medium: 8
    case .high: 1
    }
  }

  public static func profile(resourceID: String) -> CameraStreamProfile? {
    profiles.first(where: { $0.resourceID == resourceID })
  }
}

public struct CameraMeshCandidate: Equatable, Identifiable, Sendable {
  public let peerID: String
  public let routes: [String]
  public let servedProtocols: [String]
  public let expiresAt: String
  public let source: String

  public var id: String { peerID }

  public init(
    peerID: String,
    routes: [String],
    servedProtocols: [String],
    expiresAt: String,
    source: String
  ) {
    self.peerID = peerID
    self.routes = routes
    self.servedProtocols = servedProtocols
    self.expiresAt = expiresAt
    self.source = source
  }

  public init(_ candidate: AukiDiscoveryCandidate) {
    self.init(
      peerID: candidate.peerId,
      routes: candidate.routes,
      servedProtocols: candidate.servedProtocols,
      expiresAt: candidate.expiresAt,
      source: candidate.source == .ddsTracker ? "dds_tracker" : "unknown"
    )
  }
}

public struct CameraParticipantInfo: Decodable, Sendable {
  public let app: String
  public let appVersion: String
  public let name: String
  public let sessionID: String
  public let sessionClockID: String
  public let sessionClockHash: String
  public let sessionNowNs: UInt64
  public let peerID: String
  public let appInstance: String

  enum CodingKeys: String, CodingKey, CaseIterable {
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

  public init(from decoder: Decoder) throws {
    try rejectUnknownKeys(
      in: decoder,
      allowed: Set(CodingKeys.allCases.map(\.rawValue)),
      label: "Info"
    )
    let values = try decoder.container(keyedBy: CodingKeys.self)
    app = try values.decode(String.self, forKey: .app)
    appVersion = try values.decode(String.self, forKey: .appVersion)
    name = try values.decode(String.self, forKey: .name)
    sessionID = try values.decode(String.self, forKey: .sessionID)
    sessionClockID = try values.decode(String.self, forKey: .sessionClockID)
    sessionClockHash = try values.decode(String.self, forKey: .sessionClockHash)
    sessionNowNs = try values.decode(UInt64.self, forKey: .sessionNowNs)
    peerID = try values.decode(String.self, forKey: .peerID)
    appInstance = try values.decode(String.self, forKey: .appInstance)
  }
}

public struct CameraRegistryReference: Codable, Equatable, Sendable {
  public let peerID: String
  public let id: String
  public let hash: String

  enum CodingKeys: String, CodingKey {
    case peerID = "peer_id"
    case id
    case hash
  }
}

public struct CameraSensorRegistry: Decodable, Sendable {
  public let peerID: String
  public let sensorID: String
  public let kind: String
  public let type: String
  public let width: Int
  public let height: Int
  public let frameRateHz: Int
  public let imageEncoding: String
  public let pixelFormat: String
  public let rowStrideBytes: Int
  public let colorSpace: String
  public let intrinsicsModel: String
  public let distortionModel: String
  public let frame: CameraRegistryReference
  private let calibration: RejectedJSONValue?

  enum CodingKeys: String, CodingKey {
    case peerID = "peer_id"
    case sensorID = "sensor_id"
    case kind
    case type
    case width
    case height
    case frameRateHz = "frame_rate_hz"
    case imageEncoding = "image_encoding"
    case pixelFormat = "pixel_format"
    case rowStrideBytes = "row_stride_bytes"
    case colorSpace = "color_space"
    case intrinsicsModel = "intrinsics_model"
    case distortionModel = "distortion_model"
    case frame
    case calibration
  }

  fileprivate var hasCalibration: Bool { calibration != nil }
}

public struct CameraClockRegistry: Decodable, Sendable {
  public let peerID: String
  public let sessionID: String
  public let clockID: String
  public let type: String
  public let unit: String
  public let monotonic: Bool
  public let epoch: String?
  public let scope: String

  enum CodingKeys: String, CodingKey {
    case peerID = "peer_id"
    case sessionID = "session_id"
    case clockID = "clock_id"
    case type
    case unit
    case monotonic
    case epoch
    case scope
  }
}

public struct CameraFrameRegistry: Decodable, Sendable {
  public let peerID: String
  public let frameID: String
  public let handedness: String
  public let axes: [String: String]
  public let units: String

  enum CodingKeys: String, CodingKey {
    case peerID = "peer_id"
    case frameID = "frame_id"
    case handedness
    case axes
    case units
  }
}

public struct CameraCatalogMetadata: Sendable {
  public let info: CameraParticipantInfo
  public let profile: CameraStreamProfile
  public let availableQualities: [CameraQuality]
  public let sensorRef: CameraRegistryReference
  public let clockRef: CameraRegistryReference
  public let frameRef: CameraRegistryReference
  public let controlChannel: AukiMessageChannel
}

public struct CameraRemoteMetadata: Sendable {
  public let info: CameraParticipantInfo
  public let profile: CameraStreamProfile
  public let availableQualities: [CameraQuality]
  public let sensor: CameraSensorRegistry
  public let clock: CameraClockRegistry
  public let frame: CameraFrameRegistry
  public let sensorRef: CameraRegistryReference
  public let clockRef: CameraRegistryReference
  public let frameRef: CameraRegistryReference
  public let controlChannel: AukiMessageChannel
}

public struct CameraSnapshotReady: Equatable, Sendable {
  public let requestID: String
  public let sha256: String
  public let size: Int
}

public func nativeCameraTarget(
  candidate: CameraMeshCandidate,
  domainID: String
) throws -> AukiPeerTarget {
  try requireProtocols(candidate.servedProtocols)
  let normalizedDomainID = try normalizeDomainID(domainID)
  let nativeRoutes = candidate.routes.filter {
    isStructurallyValidRoute($0, peerID: candidate.peerID) && isNativeTCPRoute($0)
  }
  let circuitRoutes = nativeRoutes.filter { route in
    route.contains("/p2p-circuit/")
      && candidate.routes.contains { wss in
        isWSSRoute(wss)
          && rustValidatesRelayPair(
            tcp: route,
            wss: wss,
            peerID: candidate.peerID,
            domainID: normalizedDomainID,
            protocols: candidate.servedProtocols
          )
      }
  }
  guard
    let route = circuitRoutes.sorted().first
      ?? nativeRoutes.filter({ !$0.contains("/p2p-circuit/") }).sorted().first
  else {
    throw CameraMeshContractError("Camera publisher has no native TCP route")
  }
  return AukiPeerTarget(domainId: normalizedDomainID, peerId: candidate.peerID, route: route)
}

public func nativeCameraTarget(cardJSON: String, domainID: String) throws -> AukiPeerTarget {
  let card = try peerCardFromJson(json: cardJSON.trimmingCharacters(in: .whitespacesAndNewlines))
  guard card.domainId == domainID.lowercased() else {
    throw CameraMeshContractError("Camera publisher belongs to another Domain")
  }
  try requireProtocols(card.protocols)
  return try nativePeerTarget(card: card, requiredProtocol: CameraMeshContract.streamProtocolID)
}

public func parseCameraCatalog(
  infoJSON: String,
  catalogJSON: String,
  expectedPeerID: String,
  preferredQuality: CameraQuality = .low
) throws -> CameraCatalogMetadata {
  let info = try decode(CameraParticipantInfo.self, json: infoJSON, label: "Info")
  guard info.peerID == expectedPeerID else {
    throw CameraMeshContractError("Info authenticated the wrong peer")
  }
  guard info.app == CameraMeshContract.application else {
    throw CameraMeshContractError("Info app must be \(CameraMeshContract.application)")
  }
  guard info.appVersion == CameraMeshContract.applicationVersion else {
    throw CameraMeshContractError(
      "Info app version must be \(CameraMeshContract.applicationVersion)"
    )
  }
  try validateRegistryHash(info.sessionClockHash, label: "Info clock hash")

  let snapshot = try decode(CameraCatalogSnapshot.self, json: catalogJSON, label: "Catalog")
  let cameraRows = snapshot.resources.filter {
    $0.variant == "sensor_log" && CameraMeshContract.profile(resourceID: $0.resourceID) != nil
  }
  guard !cameraRows.isEmpty else {
    throw CameraMeshContractError("approval_required: camera Catalog row is unavailable")
  }
  let resourceIDs = cameraRows.map(\.resourceID)
  guard Set(resourceIDs).count == resourceIDs.count else {
    throw CameraMeshContractError("Camera Catalog contains duplicate resource IDs")
  }

  let cameras = try cameraRows.map { row in
    guard let profile = CameraMeshContract.profile(resourceID: row.resourceID) else {
      throw CameraMeshContractError("Camera Catalog has an unsupported rendition")
    }
    guard row.sourcePeerID == expectedPeerID, row.writerPeerID == expectedPeerID else {
      throw CameraMeshContractError("Camera Catalog source or writer does not match the peer")
    }
    guard row.state == "live" else {
      throw CameraMeshContractError("Camera Catalog resource is not live")
    }
    guard let sensor = row.sensor, sensor.kind == "camera", sensor.type == "rgb" else {
      throw CameraMeshContractError("Camera Catalog has the wrong sensor metadata")
    }
    guard let manifest = row.manifest, let frame = manifest.frame else {
      throw CameraMeshContractError("Camera Catalog is missing clock or frame metadata")
    }
    let sensorRef = CameraRegistryReference(
      peerID: expectedPeerID,
      id: sensor.sensorID,
      hash: sensor.sensorHash
    )
    try validateReference(sensorRef, owner: expectedPeerID, label: "Camera Sensor")
    try validateReference(manifest.clock, owner: expectedPeerID, label: "Camera Clock")
    try validateReference(frame, owner: expectedPeerID, label: "Camera Frame")
    guard info.sessionClockID == manifest.clock.id,
      info.sessionClockHash == manifest.clock.hash
    else {
      throw CameraMeshContractError("Info and Catalog use different clocks")
    }
    return ParsedCameraCatalogRendition(
      profile: profile,
      sensorRef: sensorRef,
      clockRef: manifest.clock,
      frameRef: frame
    )
  }.sorted { left, right in
    let leftIndex = CameraQuality.allCases.firstIndex(of: left.profile.quality) ?? 0
    let rightIndex = CameraQuality.allCases.firstIndex(of: right.profile.quality) ?? 0
    return leftIndex < rightIndex
  }
  guard
    let camera = cameras.first(where: { $0.profile.quality == preferredQuality })
      ?? cameras.first
  else {
    throw CameraMeshContractError("Camera Catalog has no supported rendition")
  }

  let controls = snapshot.resources.filter {
    $0.variant == "message_channel"
      && $0.resourceID == CameraMeshContract.controlResourceID
  }
  guard controls.count == 1, let control = controls.first,
    let owner = control.ownerPeerID, let clock = control.clock
  else {
    throw CameraMeshContractError("Camera Catalog control channel is missing or duplicated")
  }
  guard owner == expectedPeerID, clock == camera.clockRef else {
    throw CameraMeshContractError("Camera control channel owner or clock is invalid")
  }

  return CameraCatalogMetadata(
    info: info,
    profile: camera.profile,
    availableQualities: cameras.map { $0.profile.quality },
    sensorRef: camera.sensorRef,
    clockRef: camera.clockRef,
    frameRef: camera.frameRef,
    controlChannel: AukiMessageChannel(
      ownerPeerId: owner,
      resourceId: CameraMeshContract.controlResourceID,
      clock: AukiMessageClockReference(peerId: clock.peerID, id: clock.id, hash: clock.hash)
    )
  )
}

public func resolveCameraMetadata(
  catalog: CameraCatalogMetadata,
  sensorJSON: String,
  clockJSON: String,
  frameJSON: String
) throws -> CameraRemoteMetadata {
  let sensor = try decode(CameraSensorRegistry.self, json: sensorJSON, label: "Sensor Registry")
  let clock = try decode(CameraClockRegistry.self, json: clockJSON, label: "Clock Registry")
  let frame = try decode(CameraFrameRegistry.self, json: frameJSON, label: "Frame Registry")
  let owner = catalog.info.peerID

  guard sensor.peerID == owner, sensor.sensorID == catalog.sensorRef.id,
    sensor.sensorID == catalog.profile.resourceID,
    sensor.kind == "camera", sensor.type == "rgb",
    sensor.width == catalog.profile.width, sensor.height == catalog.profile.height,
    sensor.frameRateHz == catalog.profile.rateHz,
    sensor.imageEncoding == "jpeg", sensor.pixelFormat == "rgb8",
    sensor.rowStrideBytes == 0, sensor.colorSpace == "srgb",
    sensor.intrinsicsModel == "none", sensor.distortionModel == "none",
    !sensor.hasCalibration, sensor.frame == catalog.frameRef
  else {
    throw CameraMeshContractError("Camera Sensor Registry does not match the locked contract")
  }
  guard clock.peerID == owner, clock.clockID == catalog.clockRef.id,
    clock.clockID == CameraMeshContract.clockID,
    clock.sessionID == catalog.info.sessionID, clock.type == "utc_clock",
    clock.unit == "ns", !clock.monotonic,
    clock.epoch == "1970-01-01T00:00:00Z", clock.scope == "global"
  else {
    throw CameraMeshContractError("Camera Clock Registry does not match the locked contract")
  }
  guard frame.peerID == owner, frame.frameID == catalog.frameRef.id,
    frame.frameID == CameraMeshContract.frameID, frame.handedness == "right",
    frame.units == "meters", frame.axes.count == 3,
    frame.axes["x"] == "right", frame.axes["y"] == "down",
    frame.axes["z"] == "forward"
  else {
    throw CameraMeshContractError("Camera Frame Registry is not ROS optical")
  }

  return CameraRemoteMetadata(
    info: catalog.info,
    profile: catalog.profile,
    availableQualities: catalog.availableQualities,
    sensor: sensor,
    clock: clock,
    frame: frame,
    sensorRef: catalog.sensorRef,
    clockRef: catalog.clockRef,
    frameRef: catalog.frameRef,
    controlChannel: catalog.controlChannel
  )
}

public func validateStreamManifest(
  _ manifest: AukiStreamManifest,
  metadata: CameraRemoteMetadata
) throws {
  guard manifest.resourceId == metadata.profile.resourceID,
    manifest.payload == "camera_frame", manifest.writerMode == "live",
    manifest.expectedRateHz == UInt32(metadata.profile.rateHz),
    manifest.sensorId == metadata.sensorRef.id,
    manifest.sensorHash == metadata.sensorRef.hash,
    manifest.clockPeerId == metadata.clockRef.peerID,
    manifest.clockId == metadata.clockRef.id,
    manifest.clockHash == metadata.clockRef.hash,
    manifest.frameId == metadata.frameRef.id,
    manifest.frameHash == metadata.frameRef.hash,
    manifest.fromFrameId.isEmpty, manifest.fromFrameHash.isEmpty,
    manifest.toFrameId.isEmpty, manifest.toFrameHash.isEmpty,
    manifest.mapPeerId.isEmpty, manifest.mapId.isEmpty, manifest.mapHash.isEmpty
  else {
    throw CameraMeshContractError("Camera Stream manifest does not match the locked contract")
  }
}

public func makeSnapshotRequest(
  requestID: String,
  replyPeerID: String,
  replyRoutes: [String],
  clock: AukiMessageClockReference
) throws -> Data {
  try validateRequestID(requestID)
  guard !replyPeerID.isEmpty else {
    throw CameraMeshContractError("Snapshot reply Peer ID is empty")
  }
  guard !replyRoutes.isEmpty, replyRoutes.count <= 4,
    Set(replyRoutes).count == replyRoutes.count,
    replyRoutes.allSatisfy({ isStructurallyValidRoute($0, peerID: replyPeerID) })
  else {
    throw CameraMeshContractError("Snapshot reply routes are invalid")
  }
  guard clock.peerId == replyPeerID, !clock.id.isEmpty else {
    throw CameraMeshContractError("Snapshot reply clock is incomplete or not requester-owned")
  }
  let nativeRoutes = replyRoutes.filter(isNativeTCPRoute)
  let wssRoutes = replyRoutes.filter(isWSSRoute)
  guard !nativeRoutes.isEmpty, !wssRoutes.isEmpty else {
    throw CameraMeshContractError(
      "Snapshot reply routes must support both native TCP and WSS callers"
    )
  }
  try validateRegistryHash(clock.hash, label: "Snapshot reply clock hash")
  let request = SnapshotRequestWire(
    version: 1,
    requestID: requestID,
    reply: SnapshotReplyWire(
      target: SnapshotTargetWire(peerID: replyPeerID, routes: replyRoutes),
      channel: SnapshotChannelWire(
        variant: "message_channel",
        ownerPeerID: replyPeerID,
        resourceID: CameraMeshContract.replyResourceID,
        clock: CameraRegistryReference(peerID: clock.peerId, id: clock.id, hash: clock.hash)
      )
    )
  )
  let encoder = JSONEncoder()
  encoder.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
  return try encoder.encode(request)
}

public func decodeSnapshotReady(_ payload: Data) throws -> CameraSnapshotReady {
  let ready: SnapshotReadyWire
  do {
    ready = try JSONDecoder().decode(SnapshotReadyWire.self, from: payload)
  } catch {
    throw CameraMeshContractError("Invalid snapshot-ready payload: \(error.localizedDescription)")
  }
  guard ready.version == 1 else {
    throw CameraMeshContractError("Unsupported snapshot-ready version")
  }
  try validateRequestID(ready.requestID)
  guard ready.sha256.utf8.count == 64, ready.sha256.utf8.allSatisfy(isLowerHex) else {
    throw CameraMeshContractError("Snapshot-ready SHA-256 is invalid")
  }
  guard ready.size > 0, ready.size <= CameraMeshContract.maximumBlobBytes else {
    throw CameraMeshContractError("Snapshot-ready Blob size is invalid")
  }
  return CameraSnapshotReady(
    requestID: ready.requestID,
    sha256: ready.sha256,
    size: ready.size
  )
}

public func validateJPEG(_ bytes: Data) throws {
  guard bytes.count >= 4, bytes.starts(with: [0xff, 0xd8]), bytes.suffix(2) == Data([0xff, 0xd9])
  else {
    throw CameraMeshContractError("Camera frame is not a JPEG")
  }
}

public func validateJPEG(_ bytes: Data, profile: CameraStreamProfile) throws {
  try validateJPEG(bytes)
  guard
    let source = CGImageSourceCreateWithData(bytes as CFData, nil),
    let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
    let width = properties[kCGImagePropertyPixelWidth] as? NSNumber,
    let height = properties[kCGImagePropertyPixelHeight] as? NSNumber,
    width.intValue == profile.width,
    height.intValue == profile.height
  else {
    throw CameraMeshContractError(
      "Camera JPEG dimensions do not match \(profile.width)×\(profile.height)"
    )
  }
}

private struct ParsedCameraCatalogRendition {
  let profile: CameraStreamProfile
  let sensorRef: CameraRegistryReference
  let clockRef: CameraRegistryReference
  let frameRef: CameraRegistryReference
}

private struct CameraCatalogSnapshot: Decodable {
  let resources: [CameraCatalogRow]

  private enum CodingKeys: String, CodingKey, CaseIterable {
    case resources
  }

  init(from decoder: Decoder) throws {
    try rejectUnknownKeys(
      in: decoder,
      allowed: Set(CodingKeys.allCases.map(\.rawValue)),
      label: "Catalog response"
    )
    resources = try decoder.container(keyedBy: CodingKeys.self)
      .decode([CameraCatalogRow].self, forKey: .resources)
  }
}

private struct CameraCatalogRow: Decodable {
  let variant: String
  let sourcePeerID: String?
  let writerPeerID: String?
  let ownerPeerID: String?
  let resourceID: String
  let state: String?
  let sensor: CameraCatalogSensor?
  let manifest: CameraCatalogManifest?
  let clock: CameraRegistryReference?

  enum CodingKeys: String, CodingKey {
    case variant
    case sourcePeerID = "source_peer_id"
    case writerPeerID = "writer_peer_id"
    case ownerPeerID = "owner_peer_id"
    case resourceID = "resource_id"
    case state
    case sensor
    case manifest
    case clock
  }

  init(from decoder: Decoder) throws {
    let values = try decoder.container(keyedBy: CodingKeys.self)
    variant = try values.decode(String.self, forKey: .variant)
    if variant == "message_channel" {
      try rejectUnknownKeys(
        in: decoder,
        allowed: ["variant", "owner_peer_id", "resource_id", "clock"],
        label: "Message channel Catalog row"
      )
    }
    sourcePeerID = try values.decodeIfPresent(String.self, forKey: .sourcePeerID)
    writerPeerID = try values.decodeIfPresent(String.self, forKey: .writerPeerID)
    ownerPeerID = try values.decodeIfPresent(String.self, forKey: .ownerPeerID)
    resourceID = try values.decode(String.self, forKey: .resourceID)
    state = try values.decodeIfPresent(String.self, forKey: .state)
    sensor = try values.decodeIfPresent(CameraCatalogSensor.self, forKey: .sensor)
    manifest = try values.decodeIfPresent(CameraCatalogManifest.self, forKey: .manifest)
    clock = try values.decodeIfPresent(CameraRegistryReference.self, forKey: .clock)
  }
}

private struct CameraCatalogSensor: Decodable {
  let kind: String
  let type: String
  let sensorID: String
  let sensorHash: String

  enum CodingKeys: String, CodingKey {
    case kind
    case type
    case sensorID = "sensor_id"
    case sensorHash = "sensor_hash"
  }
}

private struct CameraCatalogManifest: Decodable {
  let clock: CameraRegistryReference
  let frame: CameraRegistryReference?
}

private struct SnapshotRequestWire: Encodable {
  let version: Int
  let requestID: String
  let reply: SnapshotReplyWire

  enum CodingKeys: String, CodingKey {
    case version
    case requestID = "requestId"
    case reply
  }
}

private struct SnapshotReplyWire: Encodable {
  let target: SnapshotTargetWire
  let channel: SnapshotChannelWire
}

private struct SnapshotTargetWire: Encodable {
  let peerID: String
  let routes: [String]

  enum CodingKeys: String, CodingKey {
    case peerID = "peerId"
    case routes
  }
}

private struct SnapshotChannelWire: Encodable {
  let variant: String
  let ownerPeerID: String
  let resourceID: String
  let clock: CameraRegistryReference

  enum CodingKeys: String, CodingKey {
    case variant
    case ownerPeerID = "owner_peer_id"
    case resourceID = "resource_id"
    case clock
  }
}

private struct SnapshotReadyWire: Decodable {
  let version: Int
  let requestID: String
  let sha256: String
  let size: Int

  enum CodingKeys: String, CodingKey, CaseIterable {
    case version
    case requestID = "requestId"
    case sha256
    case size
  }

  init(from decoder: Decoder) throws {
    try rejectUnknownKeys(
      in: decoder,
      allowed: Set(CodingKeys.allCases.map(\.rawValue)),
      label: "Snapshot-ready payload"
    )
    let values = try decoder.container(keyedBy: CodingKeys.self)
    version = try values.decode(Int.self, forKey: .version)
    requestID = try values.decode(String.self, forKey: .requestID)
    sha256 = try values.decode(String.self, forKey: .sha256)
    size = try values.decode(Int.self, forKey: .size)
  }
}

private struct RejectedJSONValue: Decodable, Sendable {
  init(from decoder: Decoder) throws {
    throw DecodingError.dataCorrupted(
      .init(codingPath: decoder.codingPath, debugDescription: "value must be absent")
    )
  }
}

private struct AnyCodingKey: CodingKey {
  let stringValue: String
  let intValue: Int?

  init?(stringValue: String) {
    self.stringValue = stringValue
    intValue = nil
  }

  init?(intValue: Int) {
    stringValue = String(intValue)
    self.intValue = intValue
  }
}

private func rejectUnknownKeys(
  in decoder: Decoder,
  allowed: Set<String>,
  label: String
) throws {
  let values = try decoder.container(keyedBy: AnyCodingKey.self)
  let unknown = values.allKeys.map(\.stringValue).filter { !allowed.contains($0) }.sorted()
  guard unknown.isEmpty else {
    throw DecodingError.dataCorrupted(
      .init(
        codingPath: decoder.codingPath,
        debugDescription: "\(label) contains unknown field(s): \(unknown.joined(separator: ", "))"
      )
    )
  }
}

private func decode<T: Decodable>(_ type: T.Type, json: String, label: String) throws -> T {
  guard json.utf8.count <= 1024 * 1024 else {
    throw CameraMeshContractError("\(label) JSON exceeds the application limit")
  }
  do {
    return try JSONDecoder().decode(type, from: Data(json.utf8))
  } catch {
    throw CameraMeshContractError("Invalid \(label) JSON: \(error.localizedDescription)")
  }
}

private func requireProtocols(_ protocols: [String]) throws {
  let missing = CameraMeshContract.publisherProtocolIDs.filter { !protocols.contains($0) }
  guard missing.isEmpty else {
    throw CameraMeshContractError(
      "Camera publisher does not advertise: \(missing.joined(separator: ", "))"
    )
  }
}

private func validateReference(
  _ reference: CameraRegistryReference,
  owner: String,
  label: String
) throws {
  guard reference.peerID == owner, !reference.id.isEmpty else {
    throw CameraMeshContractError("\(label) Registry reference is invalid")
  }
  try validateRegistryHash(reference.hash, label: "\(label) hash")
}

private func validateRegistryHash(_ value: String, label: String) throws {
  guard value.utf8.count == 32, value.utf8.allSatisfy(isLowerHex) else {
    throw CameraMeshContractError("\(label) must be a lowercase XXH3-128 hash")
  }
}

private func validateRequestID(_ value: String) throws {
  guard (1...128).contains(value.utf8.count),
    value.utf8.allSatisfy({ byte in
      (48...57).contains(byte) || (65...90).contains(byte) || (97...122).contains(byte)
        || byte == 46 || byte == 95 || byte == 58 || byte == 45
    })
  else {
    throw CameraMeshContractError("Snapshot requestId must be 1-128 safe ASCII characters")
  }
}

private func normalizeDomainID(_ value: String) throws -> String {
  guard let domainID = UUID(uuidString: value) else {
    throw CameraMeshContractError("Camera Domain ID is invalid")
  }
  return domainID.uuidString.lowercased()
}

private func isStructurallyValidRoute(_ route: String, peerID: String) -> Bool {
  guard !peerID.isEmpty, (1...2_048).contains(route.utf8.count), route.first == "/",
    !route.dropFirst().contains("//"),
    !route.unicodeScalars.contains(where: { CharacterSet.whitespacesAndNewlines.contains($0) })
  else {
    return false
  }
  let components = route.split(separator: "/", omittingEmptySubsequences: true).map(String.init)
  guard components.count >= 6,
    components[components.count - 2] == "p2p", components.last == peerID,
    let tcpIndex = components.firstIndex(of: "tcp"), tcpIndex + 1 < components.count,
    let port = UInt16(components[tcpIndex + 1]), port > 0
  else {
    return false
  }
  if let circuitIndex = components.firstIndex(of: "p2p-circuit") {
    guard circuitIndex >= 2, circuitIndex + 2 == components.count - 1,
      components[circuitIndex - 2] == "p2p", !components[circuitIndex - 1].isEmpty,
      components[circuitIndex + 1] == "p2p", components[circuitIndex + 2] == peerID
    else {
      return false
    }
  }
  return true
}

private func isNativeTCPRoute(_ route: String) -> Bool {
  let components = route.split(separator: "/", omittingEmptySubsequences: true)
  return components.contains("tcp") && !components.contains("ws") && !components.contains("wss")
}

private func isWSSRoute(_ route: String) -> Bool {
  route.split(separator: "/", omittingEmptySubsequences: true).contains("wss")
}

private func rustValidatesRelayPair(
  tcp: String,
  wss: String,
  peerID: String,
  domainID: String,
  protocols: [String]
) -> Bool {
  let card = AukiPeerCard(
    version: 1,
    domainId: domainID,
    peerId: peerID,
    protocols: protocols,
    routes: AukiPeerRoutes(tcp: tcp, wss: wss)
  )
  return (try? nativePeerTarget(card: card, requiredProtocol: nil)) != nil
}

private func isLowerHex(_ byte: UInt8) -> Bool {
  (48...57).contains(byte) || (97...102).contains(byte)
}

public struct CameraMeshContractError: LocalizedError, Sendable {
  public let message: String

  public init(_ message: String) {
    self.message = message
  }

  public var errorDescription: String? { message }
}
