import AukiSDK
import Foundation

public enum StandardProtocolFamily: String, CaseIterable, Sendable {
  case info
  case catalog
  case registry
  case blob
  case message
  case stream
}

public struct StandardProbeReport: Equatable, Sendable {
  public let targetPeerID: String
  public let checks: [StandardProtocolFamily: Bool]
  public let errors: [StandardProtocolFamily: String]

  public var ok: Bool {
    StandardProtocolFamily.allCases.allSatisfy { checks[$0] == true }
  }

  public init(
    targetPeerID: String,
    checks: [StandardProtocolFamily: Bool],
    errors: [StandardProtocolFamily: String]
  ) {
    self.targetPeerID = targetPeerID
    self.checks = checks
    self.errors = errors
  }
}

public struct StandardDiscoveredPeer: Equatable, Identifiable, Sendable {
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
}

public func discoveredNativePeerTarget(
  candidate: StandardDiscoveredPeer,
  domainID: String,
  requiredProtocols: [String]
) throws -> AukiPeerTarget {
  let missing = requiredProtocols.filter { !candidate.servedProtocols.contains($0) }
  guard missing.isEmpty else {
    throw StandardPlaygroundError(
      "Discovered peer \(candidate.peerID) does not advertise: \(missing.joined(separator: ", "))"
    )
  }
  let nativeRoutes = candidate.routes.filter {
      $0.contains("/tcp/") && !$0.contains("/wss/")
  }
  guard
    let tcp = nativeRoutes.first(where: { $0.contains("/p2p-circuit/") })
      ?? nativeRoutes.first
  else {
    throw StandardPlaygroundError(
      "Discovered peer \(candidate.peerID) has no native TCP route"
    )
  }
  return AukiPeerTarget(domainId: domainID, peerId: candidate.peerID, route: tcp)
}

private struct StandardRuntimeTarget {
  let domainID: String
  let peerID: String
  let protocols: [String]
  let route: String

  func exact(requiredProtocol: String) throws -> AukiPeerTarget {
    guard protocols.contains(requiredProtocol) else {
      throw StandardPlaygroundError(
        "Target does not advertise required protocol \(requiredProtocol)"
      )
    }
    return AukiPeerTarget(domainId: domainID, peerId: peerID, route: route)
  }
}

/// Thin application orchestration over the Rust-owned standard protocol bundle.
///
/// This actor owns fixture state and platform lifecycle only. Authentication,
/// exact-peer authorization, wire validation, transport, and endpoint cleanup
/// remain in the Auki SDK's Rust implementation.
public actor StandardPlayground {
  public let peerID: String
  public let domainID: String

  private let peer: AukiPeer
  private let standard: AukiStandardProtocols
  private let receiver: AukiMessageReceiver
  private let producer: AukiStreamProducer
  private var receiverTask: Task<String?, Never>?
  private var publisherTask: Task<String?, Never>?
  private var closed = false

  private init(
    peer: AukiPeer,
    standard: AukiStandardProtocols,
    receiver: AukiMessageReceiver,
    producer: AukiStreamProducer
  ) {
    self.peer = peer
    self.standard = standard
    self.receiver = receiver
    self.producer = producer
    peerID = peer.peerId()
    domainID = peer.domainId()
  }

  public static func mount(
    peer: AukiPeer,
    nodeName: String = "swift-playground"
  ) async throws -> StandardPlayground {
    let standard = try await AukiStandardProtocols.mount(peer: peer)
    var receiver: AukiMessageReceiver?
    var producer: AukiStreamProducer?
    do {
      let peerID = peer.peerId()
      try standard.info().replaceJson(
        json: StandardFixtures.json(StandardFixtures.info(peerID: peerID, nodeName: nodeName))
      )
      try standard.catalog().replaceResourcesJson(
        json: StandardFixtures.json(StandardFixtures.catalogResources(peerID: peerID))
      )
      try standard.catalog().replaceMapsJson(json: #"{"resources":[]}"#)

      let storedRegistry = try standard.registry().putJson(
        kind: .frame,
        json: StandardFixtures.json(StandardFixtures.frameRegistry(peerID: peerID))
      )
      guard
        storedRegistry.id == StandardFixtures.registryID,
        StandardFixtures.isRegistryHash(storedRegistry.hash)
      else {
        throw StandardPlaygroundError("Rust returned an invalid Frame registry fixture")
      }

      let storedBlob = try standard.blob().put(bytes: StandardFixtures.blobBytes)
      guard storedBlob.sha256 == StandardFixtures.blobSHA256 else {
        throw StandardPlaygroundError("Rust returned an unexpected Blob fixture hash")
      }

      receiver = try await standard.message().declare(
        channel: StandardFixtures.messageChannel(peerID: peerID),
        receiverCapacity: 16
      )
      producer = try await standard.stream().createProducer(
        config: AukiStreamProducerConfig(
          sourcePeerId: peerID,
          payloadKind: .scalar,
          manifest: StandardFixtures.streamManifest(),
          allowedRequesterPeerIds: []
        ))

      guard let receiver, let producer else {
        throw StandardPlaygroundError("Standard protocol fixture setup did not complete")
      }
      let playground = StandardPlayground(
        peer: peer,
        standard: standard,
        receiver: receiver,
        producer: producer
      )
      await playground.startWorkers()
      return playground
    } catch {
      if let producer { try? await producer.close() }
      if let receiver { try? await receiver.close() }
      try? await standard.close()
      throw error
    }
  }

  public func card() throws -> AukiPeerCard {
    guard !closed else { throw StandardPlaygroundError("Standard playground is closed") }
    return try standard.card()
  }

  public func cardJSON() throws -> String {
    try peerCardToJson(card: card())
  }

  public func protocols() -> [String] {
    standard.protocols()
  }

  public func discover(protocolID: String? = nil) async throws -> [StandardDiscoveredPeer] {
    guard !closed else { throw StandardPlaygroundError("Standard playground is closed") }
    let candidates: [AukiDiscoveryCandidate]
    if let protocolID, !protocolID.isEmpty {
      candidates = try await peer.discoverProtocol(protocolId: protocolID)
    } else {
      candidates = try await peer.discover()
    }
    return candidates.map { candidate in
      StandardDiscoveredPeer(
        peerID: candidate.peerId,
        routes: candidate.routes,
        servedProtocols: candidate.servedProtocols,
        expiresAt: candidate.expiresAt,
        source: candidate.source == .ddsTracker ? "dds_tracker" : "unknown"
      )
    }
  }

  public func probeAll(candidate: StandardDiscoveredPeer) async -> StandardProbeReport {
    do {
      let exact = try discoveredNativePeerTarget(
        candidate: candidate,
        domainID: domainID,
        requiredProtocols: protocols()
      )
      return await probeAll(
        target: StandardRuntimeTarget(
          domainID: domainID,
          peerID: candidate.peerID,
          protocols: candidate.servedProtocols,
          route: exact.route
        ))
    } catch {
      return failedReport(targetPeerID: candidate.peerID, error: error)
    }
  }

  /// Exercise the finite snapshots and both live protocols independently.
  /// One failed family does not prevent the remaining checks from running.
  public func probeAll(cardJSON: String) async -> StandardProbeReport {
    let card: AukiPeerCard
    do {
      card = try peerCardFromJson(json: cardJSON.trimmingCharacters(in: .whitespacesAndNewlines))
    } catch {
      return failedReport(targetPeerID: "", error: error)
    }
    guard card.domainId == domainID else {
      return failedReport(
        targetPeerID: card.peerId,
        error: StandardPlaygroundError("Target peer belongs to another Domain")
      )
    }
    return await probeAll(
      target: StandardRuntimeTarget(
        domainID: card.domainId,
        peerID: card.peerId,
        protocols: card.protocols,
        route: card.routes.tcp
      ))
  }

  private func probeAll(target: StandardRuntimeTarget) async -> StandardProbeReport {
    var checks = Dictionary(
      uniqueKeysWithValues: StandardProtocolFamily.allCases.map { ($0, false) }
    )
    var errors: [StandardProtocolFamily: String] = [:]

    do {
      try await probeInfo(target)
      checks[.info] = true
    } catch {
      errors[.info] = error.localizedDescription
    }
    do {
      try await probeCatalog(target)
      checks[.catalog] = true
    } catch {
      errors[.catalog] = error.localizedDescription
    }
    do {
      try await probeRegistry(target)
      checks[.registry] = true
    } catch {
      errors[.registry] = error.localizedDescription
    }
    do {
      try await probeBlob(target)
      checks[.blob] = true
    } catch {
      errors[.blob] = error.localizedDescription
    }
    do {
      try await probeMessage(target)
      checks[.message] = true
    } catch {
      errors[.message] = error.localizedDescription
    }
    do {
      try await probeStream(target)
      checks[.stream] = true
    } catch {
      errors[.stream] = error.localizedDescription
    }

    return StandardProbeReport(targetPeerID: target.peerID, checks: checks, errors: errors)
  }

  private func failedReport(targetPeerID: String, error: Error) -> StandardProbeReport {
    let checks = Dictionary(
      uniqueKeysWithValues: StandardProtocolFamily.allCases.map { ($0, false) }
    )
    let errors = Dictionary(
      uniqueKeysWithValues: StandardProtocolFamily.allCases.map {
        ($0, error.localizedDescription)
      }
    )
    return StandardProbeReport(targetPeerID: targetPeerID, checks: checks, errors: errors)
  }

  /// Ordered, replay-safe shutdown: live children, endpoint bundle, then peer.
  public func close() async throws {
    guard !closed else { return }
    closed = true
    var failures: [String] = []

    let publishing = publisherTask
    publisherTask = nil
    publishing?.cancel()
    do { try await producer.close() } catch { failures.append("Stream producer: \(error)") }
    if let workerError = await publishing?.value {
      failures.append("Stream publisher: \(workerError)")
    }

    let receiving = receiverTask
    receiverTask = nil
    receiving?.cancel()
    do { try await receiver.close() } catch { failures.append("Message receiver: \(error)") }
    if let workerError = await receiving?.value {
      failures.append("Message drain: \(workerError)")
    }

    do { try await standard.close() } catch { failures.append("Standard endpoints: \(error)") }
    do { try await peer.shutdown() } catch { failures.append("Auki peer: \(error)") }

    if !failures.isEmpty {
      throw StandardPlaygroundError("Ordered shutdown failed: \(failures.joined(separator: "; "))")
    }
  }

  private func startWorkers() {
    let receiver = self.receiver
    receiverTask = Task {
      while !Task.isCancelled {
        do {
          guard let event = try await receiver.next() else { return nil }
          try Self.validateMessage(event)
        } catch {
          return Task.isCancelled ? nil : error.localizedDescription
        }
      }
      return nil
    }

    let producer = self.producer
    let payload = StandardFixtures.scalarBytes()
    publisherTask = Task {
      while !Task.isCancelled {
        do {
          try await producer.push(
            timestampNs: StandardFixtures.streamTimestampNs,
            payload: payload
          )
        } catch {
          return Task.isCancelled ? nil : error.localizedDescription
        }
      }
      return nil
    }
  }

  private func probeInfo(_ target: StandardRuntimeTarget) async throws {
    let endpoint = standard.info()
    let exact = try target.exact(requiredProtocol: endpoint.protocol())
    let snapshot = try StandardFixtures.decode(
      ParticipantInfo.self,
      from: try await endpoint.client().fetchExact(target: exact)
    )
    guard
      snapshot.peerID == target.peerID,
      snapshot.app == StandardFixtures.application,
      snapshot.appVersion == StandardFixtures.applicationVersion
    else {
      throw StandardPlaygroundError("Info fixture does not match the target peer")
    }
  }

  private func probeCatalog(_ target: StandardRuntimeTarget) async throws {
    let endpoint = standard.catalog()
    let resourcesTarget = try target.exact(requiredProtocol: endpoint.resourcesProtocol())
    let resources = try StandardFixtures.decode(
      CatalogResourcesSnapshot.self,
      from: try await endpoint.client().fetchResourcesExact(
        target: resourcesTarget,
        variants: [.messageChannel]
      )
    )
    guard resources == StandardFixtures.catalogResources(peerID: target.peerID) else {
      throw StandardPlaygroundError("Catalog message-channel fixture is unexpected")
    }

    let mapsTarget = try target.exact(requiredProtocol: endpoint.mapsProtocol())
    let maps = try await endpoint.client().fetchMapsExact(target: mapsTarget)
    guard try StandardFixtures.resourcesAreEmpty(json: maps) else {
      throw StandardPlaygroundError("Catalog map fixture is not empty")
    }
  }

  private func probeRegistry(_ target: StandardRuntimeTarget) async throws {
    let endpoint = standard.registry()
    let exact = try target.exact(requiredProtocol: endpoint.protocol())
    let entries = try await endpoint.client().listExact(target: exact, kind: .frame)
    guard
      entries.count == 1,
      entries[0].id == StandardFixtures.registryID,
      StandardFixtures.isRegistryHash(entries[0].hash)
    else {
      throw StandardPlaygroundError("Registry Frame fixture is unexpected")
    }
  }

  private func probeBlob(_ target: StandardRuntimeTarget) async throws {
    let endpoint = standard.blob()
    let exact = try target.exact(requiredProtocol: endpoint.protocol())
    let receipt = try await endpoint.client().fetchExact(
      target: exact,
      sha256: StandardFixtures.blobSHA256
    )
    guard
      receipt.remotePeerId == target.peerID,
      receipt.sha256 == StandardFixtures.blobSHA256,
      receipt.bytes == StandardFixtures.blobBytes,
      receipt.relayed
    else {
      throw StandardPlaygroundError("Blob fixture receipt is unexpected")
    }
  }

  private func probeMessage(_ target: StandardRuntimeTarget) async throws {
    let endpoint = standard.message()
    let exact = try target.exact(requiredProtocol: endpoint.protocol())
    let sender = try await endpoint.client().openExact(
      target: exact,
      channel: StandardFixtures.messageChannel(peerID: target.peerID)
    )
    do {
      guard sender.remotePeer().peerId == target.peerID, sender.relayed() else {
        throw StandardPlaygroundError("Message sender metadata is unexpected")
      }
      try await sender.send(
        messageType: StandardFixtures.messageType,
        timestampNs: StandardFixtures.messageTimestampNs,
        payload: StandardFixtures.messageBytes
      )
    } catch {
      let operationError = error
      try? await sender.close()
      throw operationError
    }
    try await sender.close()
  }

  private func probeStream(_ target: StandardRuntimeTarget) async throws {
    let endpoint = standard.stream()
    let exact = try target.exact(requiredProtocol: endpoint.protocol())
    let subscription = try await endpoint.client().subscribe(
      target: exact,
      payloadKind: .scalar,
      request: AukiStreamRequest(
        sourcePeerId: target.peerID,
        resourceId: StandardFixtures.streamResourceID,
        readFrom: .latest
      )
    )
    do {
      guard subscription.payloadKind() == .scalar else {
        throw StandardPlaygroundError("Stream payload kind is not scalar")
      }
      let manifest = subscription.manifest()
      guard
        manifest.resourceId == StandardFixtures.streamResourceID,
        manifest.payload == "scalar"
      else {
        throw StandardPlaygroundError("Stream manifest is unexpected")
      }
      guard let next = try await subscription.next() else {
        throw StandardPlaygroundError("Stream ended before its fixture entry")
      }
      guard case .entry(let entry) = next else {
        throw StandardPlaygroundError("Stream returned a terminal event before its fixture")
      }
      guard
        entry.sequence == 0,
        entry.timestampNs == StandardFixtures.streamTimestampNs,
        try StandardFixtures.scalarValue(from: entry.payload) == StandardFixtures.streamValue
      else {
        throw StandardPlaygroundError("Stream scalar fixture is unexpected")
      }
    } catch {
      let operationError = error
      try? await subscription.cancel()
      throw operationError
    }
    try await subscription.cancel()
  }

  private static func validateMessage(_ event: AukiMessageEvent) throws {
    guard
      event.messageType == StandardFixtures.messageType,
      event.timestampNs == StandardFixtures.messageTimestampNs,
      event.payload == StandardFixtures.messageBytes
    else {
      throw StandardPlaygroundError("Received an invalid Message fixture")
    }
  }
}

public struct StandardPlaygroundError: LocalizedError, Sendable {
  public let message: String

  public init(_ message: String) {
    self.message = message
  }

  public var errorDescription: String? { message }
}
