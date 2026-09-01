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

  /// Exercise the finite snapshots and both live protocols independently.
  /// One failed family does not prevent the remaining checks from running.
  public func probeAll(cardJSON: String) async -> StandardProbeReport {
    var checks = Dictionary(
      uniqueKeysWithValues: StandardProtocolFamily.allCases.map { ($0, false) }
    )
    var errors: [StandardProtocolFamily: String] = [:]
    let card: AukiPeerCard
    do {
      card = try peerCardFromJson(json: cardJSON.trimmingCharacters(in: .whitespacesAndNewlines))
    } catch {
      let message = error.localizedDescription
      for family in StandardProtocolFamily.allCases { errors[family] = message }
      return StandardProbeReport(targetPeerID: "", checks: checks, errors: errors)
    }

    do {
      try await probeInfo(card)
      checks[.info] = true
    } catch {
      errors[.info] = error.localizedDescription
    }
    do {
      try await probeCatalog(card)
      checks[.catalog] = true
    } catch {
      errors[.catalog] = error.localizedDescription
    }
    do {
      try await probeRegistry(card)
      checks[.registry] = true
    } catch {
      errors[.registry] = error.localizedDescription
    }
    do {
      try await probeBlob(card)
      checks[.blob] = true
    } catch {
      errors[.blob] = error.localizedDescription
    }
    do {
      try await probeMessage(card)
      checks[.message] = true
    } catch {
      errors[.message] = error.localizedDescription
    }
    do {
      try await probeStream(card)
      checks[.stream] = true
    } catch {
      errors[.stream] = error.localizedDescription
    }

    return StandardProbeReport(targetPeerID: card.peerId, checks: checks, errors: errors)
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

  private func probeInfo(_ card: AukiPeerCard) async throws {
    let endpoint = standard.info()
    let target = try nativePeerTarget(card: card, requiredProtocol: endpoint.protocol())
    let snapshot = try StandardFixtures.decode(
      ParticipantInfo.self,
      from: try await endpoint.client().fetchExact(target: target)
    )
    guard
      snapshot.peerID == card.peerId,
      snapshot.app == StandardFixtures.application,
      snapshot.appVersion == StandardFixtures.applicationVersion
    else {
      throw StandardPlaygroundError("Info fixture does not match the target peer")
    }
  }

  private func probeCatalog(_ card: AukiPeerCard) async throws {
    let endpoint = standard.catalog()
    let resourcesTarget = try nativePeerTarget(
      card: card,
      requiredProtocol: endpoint.resourcesProtocol()
    )
    let resources = try StandardFixtures.decode(
      CatalogResourcesSnapshot.self,
      from: try await endpoint.client().fetchResourcesExact(
        target: resourcesTarget,
        variants: [.messageChannel]
      )
    )
    guard resources == StandardFixtures.catalogResources(peerID: card.peerId) else {
      throw StandardPlaygroundError("Catalog message-channel fixture is unexpected")
    }

    let mapsTarget = try nativePeerTarget(card: card, requiredProtocol: endpoint.mapsProtocol())
    let maps = try await endpoint.client().fetchMapsExact(target: mapsTarget)
    guard try StandardFixtures.resourcesAreEmpty(json: maps) else {
      throw StandardPlaygroundError("Catalog map fixture is not empty")
    }
  }

  private func probeRegistry(_ card: AukiPeerCard) async throws {
    let endpoint = standard.registry()
    let target = try nativePeerTarget(card: card, requiredProtocol: endpoint.protocol())
    let entries = try await endpoint.client().listExact(target: target, kind: .frame)
    guard
      entries.count == 1,
      entries[0].id == StandardFixtures.registryID,
      StandardFixtures.isRegistryHash(entries[0].hash)
    else {
      throw StandardPlaygroundError("Registry Frame fixture is unexpected")
    }
  }

  private func probeBlob(_ card: AukiPeerCard) async throws {
    let endpoint = standard.blob()
    let target = try nativePeerTarget(card: card, requiredProtocol: endpoint.protocol())
    let receipt = try await endpoint.client().fetchExact(
      target: target,
      sha256: StandardFixtures.blobSHA256
    )
    guard
      receipt.remotePeerId == card.peerId,
      receipt.sha256 == StandardFixtures.blobSHA256,
      receipt.bytes == StandardFixtures.blobBytes,
      receipt.relayed
    else {
      throw StandardPlaygroundError("Blob fixture receipt is unexpected")
    }
  }

  private func probeMessage(_ card: AukiPeerCard) async throws {
    let endpoint = standard.message()
    let target = try nativePeerTarget(card: card, requiredProtocol: endpoint.protocol())
    let sender = try await endpoint.client().openExact(
      target: target,
      channel: StandardFixtures.messageChannel(peerID: card.peerId)
    )
    do {
      guard sender.remotePeer().peerId == card.peerId, sender.relayed() else {
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

  private func probeStream(_ card: AukiPeerCard) async throws {
    let endpoint = standard.stream()
    let target = try nativePeerTarget(card: card, requiredProtocol: endpoint.protocol())
    let subscription = try await endpoint.client().subscribe(
      target: target,
      payloadKind: .scalar,
      request: AukiStreamRequest(
        sourcePeerId: card.peerId,
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
