import ExpoModulesCore
#if canImport(AukiSDK)
import AukiSDK
#endif

public class AukiSdkExpoModule: Module {
  #if canImport(AukiSDK)
  private var sessions: [String: AukiSession] = [:]
  private var peers: [String: AukiPeer] = [:]
  private var identities: [String: AukiPeerIdentity] = [:]
  #endif

  public func definition() -> ModuleDefinition {
    Name("AukiSdkExpo")

    AsyncFunction("loginDev") { (email: String, password: String) -> String in
      #if canImport(AukiSDK)
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
      #if canImport(AukiSDK)
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
      #if canImport(AukiSDK)
      return try await self.startPeer(sessionId: sessionId, domainId: domainId, mode: nil)
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("startPeerWithDiscovery") {
      (sessionId: String, domainId: String, mode: String) -> String in
      #if canImport(AukiSDK)
      let discovery: AukiDiscoveryMode =
        mode == "DiscoverAndAdvertise" ? .discoverAndAdvertise : .discoverOnly
      return try await self.startPeer(sessionId: sessionId, domainId: domainId, mode: discovery)
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("peerId") { (peerHandle: String) -> String in
      #if canImport(AukiSDK)
      return try self.requirePeer(peerHandle).peerId()
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("domainId") { (peerHandle: String) -> String in
      #if canImport(AukiSDK)
      return try self.requirePeer(peerHandle).domainId()
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("discover") { (peerHandle: String) -> [[String: Any]] in
      #if canImport(AukiSDK)
      let candidates = try await self.requirePeer(peerHandle).discover()
      return candidates.map(Self.mapCandidate)
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("discoverProtocol") {
      (peerHandle: String, protocolId: String) -> [[String: Any]] in
      #if canImport(AukiSDK)
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
      #if canImport(AukiSDK)
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
      #if canImport(AukiSDK)
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
        _: String,
        _: [String: String],
        _: String,
        _: String
      ) -> String in
      throw unsupported("streamSubscribeExact iOS wiring follows in a later slice")
    }

    AsyncFunction("streamNext") { (_: String) -> String? in
      throw unsupported("streamNext iOS wiring follows in a later slice")
    }

    AsyncFunction("streamCancel") { (_: String) in
      throw unsupported("streamCancel iOS wiring follows in a later slice")
    }

    AsyncFunction("shutdown") { (peerHandle: String) in
      #if canImport(AukiSDK)
      if let peer = self.peers.removeValue(forKey: peerHandle) {
        try await peer.shutdown()
      }
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }

    AsyncFunction("waitStopped") { (peerHandle: String) in
      #if canImport(AukiSDK)
      try await self.requirePeer(peerHandle).waitStopped()
      #else
      throw unsupported("AukiSDK XCFramework missing")
      #endif
    }
  }

  #if canImport(AukiSDK)
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
    [
      "peerId": candidate.peerId,
      "routes": candidate.routes,
      "servedProtocols": candidate.servedProtocols,
      "expiresAt": candidate.expiresAt,
      "source": String(describing: candidate.source),
    ]
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
  #endif

  private func newId(_ prefix: String) -> String {
    "\(prefix)_\(UUID().uuidString)"
  }
}

private func unsupported(_ message: String) -> Exception {
  Exception(name: "AukiSdkExpoUnsupported", description: message)
}
