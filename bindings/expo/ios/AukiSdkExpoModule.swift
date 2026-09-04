import ExpoModulesCore
#if canImport(auki_sdk_swiftFFI)
import auki_sdk_swiftFFI
#endif

public class AukiSdkExpoModule: Module {
  // UniFFI Swift (ios/AukiSDK) is compiled into this pod; FFI comes from
  // Frameworks/AukiSDK.xcframework. canImport(auki_sdk_swiftFFI) gates the API.
  #if canImport(auki_sdk_swiftFFI)
  private var sessions: [String: AukiSession] = [:]
  private var peers: [String: AukiPeer] = [:]
  private var identities: [String: AukiPeerIdentity] = [:]
  private var streams: [String: AukiStreamSubscription] = [:]
  #endif

  public func definition() -> ModuleDefinition {
    Name("AukiSdkExpo")

    AsyncFunction("loginDev") { (email: String, password: String) -> String in
      #if canImport(auki_sdk_swiftFFI)
      let session = try await AukiSession.loginDev(email: email, password: password)
      let id = self.newId("session")
      self.sessions[id] = session
      return id
      #else
      throw unsupported("AukiSDK XCFramework missing; run scripts/sync-ios-xcframework.sh")
      #endif
    }

    AsyncFunction("loginWithDomainAccessToken") {
      (
        _: String,
        _: String,
        _: String,
        _: String
      ) -> String in
      throw unsupported(
        "loginWithDomainAccessToken is not exported on auki-sdk-swift yet; use web or loginDev on iOS"
      )
    }

    AsyncFunction("accessibleDomains") { (sessionId: String) -> [[String: Any?]] in
      #if canImport(auki_sdk_swiftFFI)
      guard let session = self.sessions[sessionId] else {
        throw unsupported("unknown session: \(sessionId)")
      }
      let domains = try await session.accessibleDomains()
      return domains.map { domain in
        [
          "id": domain.id,
          "name": domain.name,
          "description": domain.description,
          "organizationId": domain.organizationId,
        ]
      }
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("startPeer") { (sessionId: String, domainId: String) -> String in
      #if canImport(auki_sdk_swiftFFI)
      return try await self.startPeer(sessionId: sessionId, domainId: domainId, mode: nil)
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("startPeerWithDiscovery") {
      (sessionId: String, domainId: String, mode: String) -> String in
      #if canImport(auki_sdk_swiftFFI)
      let discovery: AukiDiscoveryMode =
        mode == "DiscoverAndAdvertise" ? .discoverAndAdvertise : .discoverOnly
      return try await self.startPeer(sessionId: sessionId, domainId: domainId, mode: discovery)
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("peerId") { (peerHandle: String) -> String in
      #if canImport(auki_sdk_swiftFFI)
      return try self.requirePeer(peerHandle).peerId()
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("domainId") { (peerHandle: String) -> String in
      #if canImport(auki_sdk_swiftFFI)
      return try self.requirePeer(peerHandle).domainId()
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("discover") { (peerHandle: String) -> [[String: Any]] in
      #if canImport(auki_sdk_swiftFFI)
      let candidates = try await self.requirePeer(peerHandle).discover()
      return candidates.map(Self.mapCandidate)
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("discoverProtocol") {
      (peerHandle: String, protocolId: String) -> [[String: Any]] in
      #if canImport(auki_sdk_swiftFFI)
      let candidates = try await self.requirePeer(peerHandle).discoverProtocol(
        protocolId: protocolId
      )
      return candidates.map(Self.mapCandidate)
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("infoFetchExact") {
      (peerHandle: String, target: [String: String]) -> String in
      #if canImport(auki_sdk_swiftFFI)
      let peer = try self.requirePeer(peerHandle)
      let exact = try Self.exactTarget(target, domainId: peer.domainId())
      let json = try await AukiInfoClient(peer: peer).fetchExact(target: exact)
      return json
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("catalogFetchResourcesExact") {
      (peerHandle: String, target: [String: String], variants: [String]) -> String in
      #if canImport(auki_sdk_swiftFFI)
      let peer = try self.requirePeer(peerHandle)
      let exact = try Self.exactTarget(target, domainId: peer.domainId())
      let json = try await AukiCatalogClient(peer: peer).fetchResourcesExact(
        target: exact,
        variants: []
      )
      _ = variants
      return json
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("registryListExact") {
      (_: String, _: [String: String], _: String) -> String in
      throw unsupported("registryListExact is web-only in this slice")
    }

    AsyncFunction("registryFetchExact") {
      (_: String, _: [String: String], _: String, _: String, _: String) -> String in
      throw unsupported("registryFetchExact is web-only in this slice")
    }

    AsyncFunction("blobFetchExact") {
      (_: String, _: [String: String], _: String) -> String in
      throw unsupported("blobFetchExact is web-only in this slice")
    }

    AsyncFunction("streamSubscribeExact") {
      (
        peerHandle: String,
        target: [String: String],
        payloadKind: String,
        requestJson: String
      ) -> String in
      #if canImport(auki_sdk_swiftFFI)
      let peer = try self.requirePeer(peerHandle)
      let exact = try Self.exactTarget(target, domainId: peer.domainId())
      let kind = try Self.streamPayloadKind(payloadKind)
      // Web Wasm uses `from`; UniFFI AukiStreamRequest uses `readFrom`.
      let request = try streamRequestFromJson(
        json: Self.normalizeStreamRequestJson(requestJson)
      )
      let subscription = try await AukiStreamClient(peer: peer).subscribe(
        target: exact,
        payloadKind: kind,
        request: request
      )
      let id = self.newId("stream")
      self.streams[id] = subscription
      return id
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("streamNext") { (subscriptionId: String) -> String? in
      #if canImport(auki_sdk_swiftFFI)
      guard let subscription = self.streams[subscriptionId] else {
        throw unsupported("unknown stream subscription: \(subscriptionId)")
      }
      guard let next = try await subscription.next() else {
        return nil
      }
      return try Self.encodeStreamNext(next)
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("streamCancel") { (subscriptionId: String) in
      #if canImport(auki_sdk_swiftFFI)
      guard let subscription = self.streams.removeValue(forKey: subscriptionId) else {
        return
      }
      try await subscription.cancel()
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("shutdown") { (peerHandle: String) in
      #if canImport(auki_sdk_swiftFFI)
      if let peer = self.peers.removeValue(forKey: peerHandle) {
        try await peer.shutdown()
      }
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("waitStopped") { (peerHandle: String) in
      #if canImport(auki_sdk_swiftFFI)
      try await self.requirePeer(peerHandle).waitStopped()
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }
  }

  #if canImport(auki_sdk_swiftFFI)
  private func startPeer(
    sessionId: String,
    domainId: String,
    mode: AukiDiscoveryMode?
  ) async throws -> String {
    guard let session = sessions[sessionId] else {
      throw unsupported("unknown session: \(sessionId)")
    }
    let identity = AukiPeerIdentity.generate()
    let peer: AukiPeer
    if let mode {
      peer = try await session.startPeerWithDiscovery(
        domainId: domainId,
        identity: identity,
        mode: mode
      )
    } else {
      peer = try await session.startPeer(domainId: domainId, identity: identity)
    }
    let id = newId("peer")
    peers[id] = peer
    identities[id] = identity
    return id
  }

  private func requirePeer(_ peerHandle: String) throws -> AukiPeer {
    guard let peer = peers[peerHandle] else {
      throw unsupported("unknown peer: \(peerHandle)")
    }
    return peer
  }

  private static func mapCandidate(_ candidate: AukiDiscoveryCandidate) -> [String: Any] {
    var mapped: [String: Any] = [
      "peerId": candidate.peerId,
      "routes": candidate.routes,
      "servedProtocols": candidate.servedProtocols,
      "expiresAt": candidate.expiresAt,
      "source": String(describing: candidate.source),
    ]
    if let subjectId = candidate.subjectId {
      mapped["subjectId"] = subjectId
    }
    if let peerType = candidate.peerType {
      mapped["peerType"] = peerType
    }
    return mapped
  }

  private static func exactTarget(
    _ target: [String: String],
    domainId: String
  ) throws -> AukiPeerTarget {
    guard let peerId = target["peerId"], let route = target["route"] else {
      throw unsupported("exact target requires peerId and route")
    }
    return AukiPeerTarget(
      domainId: target["domainId"] ?? domainId,
      peerId: peerId,
      route: route
    )
  }

  private static func streamPayloadKind(_ raw: String) throws -> AukiStreamPayloadKind {
    switch raw {
    case "pose":
      return .pose
    case "joint_encoders":
      return .jointEncoders
    case "camera":
      return .camera
    case "point_cloud":
      return .pointCloud
    case "audio":
      return .audio
    case "scalar":
      return .scalar
    case "detection":
      return .detection
    case "map":
      return .map
    default:
      throw unsupported("unsupported stream payloadKind: \(raw)")
    }
  }

  /// Map web-shaped Stream request JSON (`from`) onto UniFFI (`readFrom`).
  private static func normalizeStreamRequestJson(_ json: String) throws -> String {
    guard let data = json.data(using: .utf8) else {
      throw unsupported("stream request JSON is not UTF-8")
    }
    guard var object = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
      throw unsupported("stream request JSON must be an object")
    }
    if object["readFrom"] == nil, let from = object["from"] {
      object["readFrom"] = from
      object.removeValue(forKey: "from")
    }
    if object["readFrom"] == nil {
      object["readFrom"] = ["kind": "latest"]
    }
    return try jsonString(object)
  }

  private static func encodeStreamNext(_ next: AukiStreamNext) throws -> String {
    switch next {
    case .entry(let entry):
      return try jsonString([
        "kind": "entry",
        "entry": [
          "timestampNs": String(entry.timestampNs),
          "sequence": String(entry.sequence),
          "payloadBase64": entry.payload.base64EncodedString(),
        ] as [String: Any],
      ])
    case .end(let reason):
      return try jsonString([
        "kind": "end",
        "reason": encodeEndReason(reason),
        "entry": NSNull(),
      ])
    }
  }

  private static func encodeEndReason(_ reason: AukiStreamEndReason) -> [String: Any] {
    switch reason {
    case .sourceEnded:
      return ["kind": "source_ended"]
    case .producerShuttingDown:
      return ["kind": "producer_shutting_down"]
    case .sessionEnded:
      return ["kind": "session_ended"]
    case .producerError(let detail):
      return ["kind": "producer_error", "detail": detail]
    }
  }

  private static func jsonString(_ object: [String: Any]) throws -> String {
    let data = try JSONSerialization.data(withJSONObject: object)
    guard let string = String(data: data, encoding: .utf8) else {
      throw unsupported("failed to encode stream next JSON")
    }
    return string
  }
  #endif

  private func newId(_ prefix: String) -> String {
    "\(prefix)_\(UUID().uuidString)"
  }
}

private func unsupported(_ message: String) -> Exception {
  Exception(name: "AukiSdkExpoUnsupported", description: message)
}
