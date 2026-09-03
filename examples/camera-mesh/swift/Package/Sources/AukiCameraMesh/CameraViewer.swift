import AukiSDK
import Foundation

public struct CameraViewerFrame: Equatable, Sendable {
  public let jpeg: Data
  public let sequence: UInt64
  public let timestampNs: Int64

  public init(jpeg: Data, sequence: UInt64, timestampNs: Int64) {
    self.jpeg = jpeg
    self.sequence = sequence
    self.timestampNs = timestampNs
  }
}

public struct CameraViewerSnapshot: Equatable, Sendable {
  public let requestID: String
  public let sha256: String
  public let jpeg: Data
  public let relayed: Bool

  public init(requestID: String, sha256: String, jpeg: Data, relayed: Bool) {
    self.requestID = requestID
    self.sha256 = sha256
    self.jpeg = jpeg
    self.relayed = relayed
  }
}

public struct CameraViewerConnection: Equatable, Sendable {
  public let peerID: String
  public let name: String
  public let runtime: String

  public init(peerID: String, name: String, runtime: String) {
    self.peerID = peerID
    self.name = name
    self.runtime = runtime
  }
}

/// An opaque identity for one camera connection lifetime.
///
/// Compare this value across events to discard buffered work from a camera that
/// has since been disconnected or replaced.
public struct CameraViewerConnectionID: Equatable, Hashable, Sendable {
  fileprivate let generation: UInt64
}

public enum CameraViewerEvent: Equatable, Sendable {
  case status(connectionID: CameraViewerConnectionID?, message: String)
  case connected(connectionID: CameraViewerConnectionID, camera: CameraViewerConnection)
  case frame(connectionID: CameraViewerConnectionID, frame: CameraViewerFrame)
  case snapshot(connectionID: CameraViewerConnectionID, snapshot: CameraViewerSnapshot)
  case disconnected(connectionID: CameraViewerConnectionID, reason: String)
  case failed(connectionID: CameraViewerConnectionID?, message: String)
}

private struct ActiveCamera: Sendable {
  let connectionID: CameraViewerConnectionID
  let target: AukiPeerTarget
  let metadata: CameraRemoteMetadata
}

private struct PendingSnapshot: Sendable {
  let connectionID: CameraViewerConnectionID
  let target: AukiPeerTarget
  let publisherPeerID: String
  var timeout: Task<Void, Never>?
}

/// Foreground-only Camera Mesh viewer composed from the SDK's standard protocols.
///
/// Transport, authenticated exact-peer connections, protocol framing, Registry
/// hashes, Blob hashes, and CameraFrame protobuf decoding stay in Rust. This actor
/// owns only the Camera Mesh application sequence and Apple lifecycle.
public actor CameraViewer {
  public nonisolated let peerID: String
  public nonisolated let domainID: String
  public nonisolated let events: AsyncStream<CameraViewerEvent>

  private let eventContinuation: AsyncStream<CameraViewerEvent>.Continuation
  private let peer: AukiPeer
  private let info: AukiInfoEndpoint
  private let catalog: AukiCatalogEndpoint
  private let registry: AukiRegistryEndpoint
  private let blob: AukiBlobEndpoint
  private let message: AukiMessageEndpoint
  private let replyReceiver: AukiMessageReceiver
  private let replyClock: AukiMessageClockReference
  private let streamClient: AukiStreamClient

  private var receiverTask: Task<Void, Never>?
  private var streamTask: Task<Void, Never>?
  private var subscription: AukiStreamSubscription?
  private var activeCamera: ActiveCamera?
  private var pendingSnapshots: [String: PendingSnapshot] = [:]
  private var connectionGeneration: UInt64 = 0
  private var closed = false

  private init(
    peer: AukiPeer,
    info: AukiInfoEndpoint,
    catalog: AukiCatalogEndpoint,
    registry: AukiRegistryEndpoint,
    blob: AukiBlobEndpoint,
    message: AukiMessageEndpoint,
    replyReceiver: AukiMessageReceiver,
    replyClock: AukiMessageClockReference,
    events: AsyncStream<CameraViewerEvent>,
    eventContinuation: AsyncStream<CameraViewerEvent>.Continuation
  ) {
    self.peer = peer
    self.info = info
    self.catalog = catalog
    self.registry = registry
    self.blob = blob
    self.message = message
    self.replyReceiver = replyReceiver
    self.replyClock = replyClock
    self.events = events
    self.eventContinuation = eventContinuation
    peerID = peer.peerId()
    domainID = peer.domainId()
    streamClient = AukiStreamClient(peer: peer)
  }

  public static func mount(
    peer: AukiPeer,
    displayName: String = "iOS Camera Viewer"
  ) async throws -> CameraViewer {
    var info: AukiInfoEndpoint?
    var catalog: AukiCatalogEndpoint?
    var registry: AukiRegistryEndpoint?
    var blob: AukiBlobEndpoint?
    var message: AukiMessageEndpoint?
    var receiver: AukiMessageReceiver?

    do {
      let mountedInfo = try await AukiInfoEndpoint.mount(peer: peer)
      info = mountedInfo
      let mountedCatalog = try await AukiCatalogEndpoint.mount(peer: peer)
      catalog = mountedCatalog
      let mountedRegistry = try await AukiRegistryEndpoint.mount(peer: peer)
      registry = mountedRegistry
      let mountedBlob = try await AukiBlobEndpoint.mount(peer: peer)
      blob = mountedBlob
      let mountedMessage = try await AukiMessageEndpoint.mount(peer: peer)
      message = mountedMessage

      let sessionID = UUID().uuidString.lowercased()
      let clockJSON = try encodeJSON(
        LocalClock(
          peerID: peer.peerId(),
          sessionID: sessionID,
          clockID: CameraMeshContract.clockID,
          type: "utc_clock",
          unit: "ns",
          monotonic: false,
          epoch: "1970-01-01T00:00:00Z",
          scope: "global"
        ))
      let storedClock = try mountedRegistry.putJson(kind: .clock, json: clockJSON)
      let replyClock = AukiMessageClockReference(
        peerId: peer.peerId(),
        id: storedClock.id,
        hash: storedClock.hash
      )

      try mountedInfo.replaceJson(
        json: encodeJSON(
          LocalInfo(
            app: CameraMeshContract.application,
            appVersion: CameraMeshContract.applicationVersion,
            name: displayName,
            sessionID: sessionID,
            sessionClockID: storedClock.id,
            sessionClockHash: storedClock.hash,
            sessionNowNs: utcNowNs(),
            peerID: peer.peerId(),
            appInstance: "swift/viewer"
          )))
      try mountedCatalog.replaceResourcesJson(json: #"{"resources":[]}"#)
      try mountedCatalog.replaceMapsJson(json: #"{"resources":[]}"#)

      let declared = try await mountedMessage.declare(
        channel: AukiMessageChannel(
          ownerPeerId: peer.peerId(),
          resourceId: CameraMeshContract.replyResourceID,
          clock: replyClock
        ),
        receiverCapacity: 16
      )
      receiver = declared

      let stream = AsyncStream<CameraViewerEvent>.makeStream(
        bufferingPolicy: .bufferingNewest(64)
      )
      let viewer = CameraViewer(
        peer: peer,
        info: mountedInfo,
        catalog: mountedCatalog,
        registry: mountedRegistry,
        blob: mountedBlob,
        message: mountedMessage,
        replyReceiver: declared,
        replyClock: replyClock,
        events: stream.stream,
        eventContinuation: stream.continuation
      )
      await viewer.startReplyWorker()
      await viewer.emit(.status(connectionID: nil, message: "Camera Mesh viewer is ready"))
      return viewer
    } catch {
      if let receiver { try? await receiver.close() }
      if let message { try? await message.close() }
      if let blob { try? await blob.close() }
      if let registry { try? await registry.close() }
      if let catalog { try? await catalog.close() }
      if let info { try? await info.close() }
      throw error
    }
  }

  public func cardJSON() throws -> String {
    try ensureOpen("build peer card")
    return try peerCardToJson(
      card: peer.card(protocols: CameraMeshContract.viewerProtocolIDs)
    )
  }

  public func discoverCameras() async throws -> [CameraMeshCandidate] {
    try ensureOpen("discover cameras")
    return try await peer.discoverProtocol(protocolId: CameraMeshContract.streamProtocolID)
      .map(CameraMeshCandidate.init)
  }

  public func connect(candidate: CameraMeshCandidate) async throws {
    let target = try nativeCameraTarget(candidate: candidate, domainID: domainID)
    try await connect(target: target)
  }

  public func connect(cardJSON: String) async throws {
    let target = try nativeCameraTarget(cardJSON: cardJSON, domainID: domainID)
    try await connect(target: target)
  }

  public func pause() async throws {
    let remote = try active()
    try await sendControl(type: "camera.pause", payload: Data(), remote: remote)
    try ensureCurrent(remote, operation: "pause camera")
    emit(
      .status(
        connectionID: remote.connectionID,
        message: "Pause acknowledged by \(remote.target.peerId)"
      ))
  }

  public func resume() async throws {
    let remote = try active()
    try await sendControl(type: "camera.resume", payload: Data(), remote: remote)
    try ensureCurrent(remote, operation: "resume camera")
    emit(
      .status(
        connectionID: remote.connectionID,
        message: "Resume acknowledged by \(remote.target.peerId)"
      ))
  }

  @discardableResult
  public func requestSnapshot() async throws -> String {
    try ensureOpen("request snapshot")
    guard pendingSnapshots.count < CameraMeshContract.maximumPendingSnapshots else {
      throw CameraViewerError("Too many snapshot requests are pending")
    }
    let remote = try active()
    let requestID = UUID().uuidString.lowercased()
    let routes = try peer.routes()
    let payload = try makeSnapshotRequest(
      requestID: requestID,
      replyPeerID: peerID,
      replyRoutes: [routes.tcp, routes.wss],
      clock: replyClock
    )

    pendingSnapshots[requestID] = PendingSnapshot(
      connectionID: remote.connectionID,
      target: remote.target,
      publisherPeerID: remote.target.peerId,
      timeout: nil
    )

    do {
      try await sendControl(
        type: "camera.request_snapshot",
        payload: payload,
        remote: remote
      )
      try ensureCurrent(remote, operation: "request snapshot")
      armSnapshotTimeout(requestID, remote: remote)
      emit(
        .status(
          connectionID: remote.connectionID,
          message: "Snapshot \(requestID) requested; waiting for its verified Blob"
        ))
      return requestID
    } catch {
      removePendingSnapshot(requestID, connectionID: remote.connectionID)
      throw error
    }
  }

  public func disconnect(reason: String = "Camera disconnected") async {
    await disconnectActive(reason: reason, emitEvent: true)
  }

  /// Replay-safe ordered cleanup: Stream, Message receiver/endpoints, then peer.
  public func close() async throws {
    guard !closed else { return }
    closed = true
    var failures: [String] = []

    await disconnectActive(reason: "Viewer stopped", emitEvent: false)

    let receiving = receiverTask
    receiverTask = nil
    receiving?.cancel()
    do { try await replyReceiver.close() } catch { failures.append("Message receiver: \(error)") }
    await receiving?.value

    do { try await message.close() } catch { failures.append("Message endpoint: \(error)") }
    do { try await blob.close() } catch { failures.append("Blob endpoint: \(error)") }
    do { try await registry.close() } catch { failures.append("Registry endpoint: \(error)") }
    do { try await catalog.close() } catch { failures.append("Catalog endpoint: \(error)") }
    do { try await info.close() } catch { failures.append("Info endpoint: \(error)") }
    do { try await peer.shutdown() } catch { failures.append("Auki peer: \(error)") }

    eventContinuation.finish()
    if !failures.isEmpty {
      throw CameraViewerError("Ordered shutdown failed: \(failures.joined(separator: "; "))")
    }
  }

  private func connect(target: AukiPeerTarget) async throws {
    try ensureOpen("connect camera")
    await disconnectActive(reason: "Switching camera", emitEvent: false)
    connectionGeneration &+= 1
    let connectionID = CameraViewerConnectionID(generation: connectionGeneration)
    emit(
      .status(
        connectionID: connectionID,
        message: "Authenticating camera \(target.peerId)…"
      ))

    let infoJSON = try await info.client().fetchExact(target: target)
    let catalogJSON = try await catalog.client().fetchResourcesExact(
      target: target,
      variants: [.sensorLog, .messageChannel]
    )
    let catalogMetadata = try parseCameraCatalog(
      infoJSON: infoJSON,
      catalogJSON: catalogJSON,
      expectedPeerID: target.peerId
    )
    let registryClient = registry.client()
    let sensorJSON = try await registryClient.fetchExact(
      target: target,
      kind: .sensor,
      id: catalogMetadata.sensorRef.id,
      hash: catalogMetadata.sensorRef.hash
    )
    let clockJSON = try await registryClient.fetchExact(
      target: target,
      kind: .clock,
      id: catalogMetadata.clockRef.id,
      hash: catalogMetadata.clockRef.hash
    )
    let frameJSON = try await registryClient.fetchExact(
      target: target,
      kind: .frame,
      id: catalogMetadata.frameRef.id,
      hash: catalogMetadata.frameRef.hash
    )
    let metadata = try resolveCameraMetadata(
      catalog: catalogMetadata,
      sensorJSON: sensorJSON,
      clockJSON: clockJSON,
      frameJSON: frameJSON
    )
    let opened = try await streamClient.subscribe(
      target: target,
      payloadKind: .camera,
      request: AukiStreamRequest(
        sourcePeerId: target.peerId,
        resourceId: CameraMeshContract.cameraResourceID,
        readFrom: .latest
      )
    )

    do {
      try validateStreamManifest(opened.manifest(), metadata: metadata)
      guard !closed, connectionGeneration == connectionID.generation else {
        try await opened.cancel()
        throw CameraViewerError("Camera connection was cancelled")
      }
      activeCamera = ActiveCamera(
        connectionID: connectionID,
        target: target,
        metadata: metadata
      )
      subscription = opened
      let info = metadata.info
      emit(
        .connected(
          connectionID: connectionID,
          camera: CameraViewerConnection(
            peerID: target.peerId,
            name: info.name,
            runtime: info.appInstance
          )))
      startStreamWorker(opened, connectionID: connectionID)
    } catch {
      try? await opened.cancel()
      throw error
    }
  }

  private func sendControl(
    type: String,
    payload: Data,
    remote: ActiveCamera
  ) async throws {
    try ensureOpen("send camera control")
    try ensureCurrent(remote, operation: "send camera control")
    let sender: AukiMessageSender
    do {
      sender = try await message.client().openExact(
        target: remote.target,
        channel: remote.metadata.controlChannel
      )
    } catch {
      try ensureCurrent(remote, operation: "send camera control")
      throw error
    }
    do {
      try ensureCurrent(remote, operation: "send camera control")
      guard sender.remotePeer().peerId == remote.target.peerId else {
        throw CameraViewerError("Message authenticated the wrong camera peer")
      }
      try await sender.send(messageType: type, timestampNs: utcNowNs(), payload: payload)
      try ensureCurrent(remote, operation: "send camera control")
    } catch {
      let operationError = error
      try? await sender.close()
      try ensureCurrent(remote, operation: "send camera control")
      throw operationError
    }
    do {
      try await sender.close()
    } catch {
      try ensureCurrent(remote, operation: "send camera control")
      throw error
    }
    try ensureCurrent(remote, operation: "send camera control")
  }

  private func startReplyWorker() {
    let receiver = replyReceiver
    receiverTask = Task { [weak self] in
      while !Task.isCancelled {
        do {
          guard let event = try await receiver.next() else { return }
          await self?.handleSnapshotEvent(event)
        } catch {
          guard !Task.isCancelled else { return }
          await self?.emit(
            .failed(
              connectionID: nil,
              message: "Snapshot receiver failed: \(error.localizedDescription)"
            ))
          return
        }
      }
    }
  }

  private func startStreamWorker(
    _ opened: AukiStreamSubscription,
    connectionID: CameraViewerConnectionID
  ) {
    streamTask = Task { [weak self] in
      do {
        while !Task.isCancelled {
          guard let next = try await opened.next() else {
            await self?.streamEnded(
              "Camera stream closed",
              opened: opened,
              connectionID: connectionID
            )
            return
          }
          switch next {
          case .entry(let entry):
            let jpeg = try decodeCameraFrameImage(payload: entry.payload)
            try validateJPEG(jpeg)
            await self?.receivedFrame(
              CameraViewerFrame(
                jpeg: jpeg,
                sequence: entry.sequence,
                timestampNs: entry.timestampNs
              ),
              connectionID: connectionID
            )
          case .end(let reason):
            await self?.streamEnded(
              "Camera stream ended: \(describe(reason))",
              opened: opened,
              connectionID: connectionID
            )
            return
          }
        }
      } catch {
        guard !Task.isCancelled else { return }
        await self?.streamEnded(
          "Camera stream failed: \(error.localizedDescription)",
          opened: opened,
          connectionID: connectionID,
          failed: true
        )
      }
    }
  }

  private func receivedFrame(
    _ frame: CameraViewerFrame,
    connectionID: CameraViewerConnectionID
  ) {
    guard isCurrent(connectionID), subscription != nil else { return }
    emit(.frame(connectionID: connectionID, frame: frame))
  }

  private func streamEnded(
    _ reason: String,
    opened: AukiStreamSubscription,
    connectionID: CameraViewerConnectionID,
    failed: Bool = false
  ) async {
    guard isCurrent(connectionID) else { return }
    try? await opened.cancel()
    guard isCurrent(connectionID) else { return }
    streamTask = nil
    subscription = nil
    activeCamera = nil
    cancelPendingSnapshots(connectionID: connectionID)
    if failed {
      emit(.failed(connectionID: connectionID, message: reason))
    } else {
      emit(.disconnected(connectionID: connectionID, reason: reason))
    }
  }

  private func handleSnapshotEvent(_ event: AukiMessageEvent) async {
    guard event.messageType == "camera.snapshot_ready" else {
      emit(
        .status(
          connectionID: nil,
          message: "Ignored unsupported camera reply \(event.messageType)"
        ))
      return
    }
    guard event.sender.domainIds.contains(domainID) else {
      emit(
        .status(
          connectionID: nil,
          message: "Ignored a snapshot reply from another Domain"
        ))
      return
    }

    let ready: CameraSnapshotReady
    do {
      ready = try decodeSnapshotReady(event.payload)
    } catch {
      emit(
        .status(
          connectionID: nil,
          message: "Ignored an invalid snapshot reply: \(error.localizedDescription)"
        ))
      return
    }

    guard let pending = pendingSnapshots[ready.requestID] else {
      emit(
        .status(
          connectionID: nil,
          message: "Ignored a snapshot reply with no pending request"
        ))
      return
    }

    do {
      try ensureCurrent(
        pending.connectionID,
        peerID: pending.publisherPeerID,
        operation: "receive snapshot"
      )
      guard event.sender.peerId == pending.publisherPeerID else {
        throw CameraViewerError("Snapshot reply came from the wrong peer")
      }
      removePendingSnapshot(ready.requestID, connectionID: pending.connectionID)

      let receipt = try await blob.client().fetchExact(
        target: pending.target,
        sha256: ready.sha256
      )
      try ensureCurrent(
        pending.connectionID,
        peerID: pending.publisherPeerID,
        operation: "receive snapshot"
      )
      guard receipt.remotePeerId == pending.publisherPeerID else {
        throw CameraViewerError("Blob authenticated the wrong camera peer")
      }
      guard receipt.sha256 == ready.sha256 else {
        throw CameraViewerError("Blob receipt returned a different hash")
      }
      guard receipt.bytes.count == ready.size else {
        throw CameraViewerError("Snapshot Blob size does not match its announcement")
      }
      try validateJPEG(receipt.bytes)
      emit(
        .snapshot(
          connectionID: pending.connectionID,
          snapshot: CameraViewerSnapshot(
            requestID: ready.requestID,
            sha256: ready.sha256,
            jpeg: receipt.bytes,
            relayed: receipt.relayed
          )))
    } catch {
      guard isCurrent(pending.connectionID, peerID: pending.publisherPeerID) else { return }
      emit(
        .failed(
          connectionID: pending.connectionID,
          message: "Snapshot reply failed: \(error.localizedDescription)"
        ))
    }
  }

  private func expireSnapshot(
    _ requestID: String,
    connectionID: CameraViewerConnectionID
  ) {
    guard
      let pending = pendingSnapshots[requestID],
      pending.connectionID == connectionID
    else { return }
    pendingSnapshots.removeValue(forKey: requestID)
    guard isCurrent(connectionID, peerID: pending.publisherPeerID) else { return }
    emit(
      .failed(
        connectionID: connectionID,
        message: "Snapshot \(requestID) timed out"
      ))
  }

  private func armSnapshotTimeout(
    _ requestID: String,
    remote: ActiveCamera
  ) {
    guard
      isCurrent(remote.connectionID, peerID: remote.target.peerId),
      pendingSnapshots[requestID]?.connectionID == remote.connectionID
    else { return }
    let connectionID = remote.connectionID
    let timeout = Task { [weak self] in
      try? await Task.sleep(for: .seconds(CameraMeshContract.snapshotTimeoutSeconds))
      guard !Task.isCancelled else { return }
      await self?.expireSnapshot(requestID, connectionID: connectionID)
    }
    pendingSnapshots[requestID]?.timeout = timeout
  }

  private func removePendingSnapshot(
    _ requestID: String,
    connectionID: CameraViewerConnectionID
  ) {
    guard pendingSnapshots[requestID]?.connectionID == connectionID else { return }
    pendingSnapshots.removeValue(forKey: requestID)?.timeout?.cancel()
  }

  private func cancelPendingSnapshots(connectionID: CameraViewerConnectionID) {
    let requestIDs = pendingSnapshots.compactMap { requestID, pending in
      pending.connectionID == connectionID ? requestID : nil
    }
    for requestID in requestIDs {
      removePendingSnapshot(requestID, connectionID: connectionID)
    }
  }

  private func disconnectActive(reason: String, emitEvent: Bool) async {
    connectionGeneration &+= 1
    let invalidatedGeneration = connectionGeneration
    let disconnectedConnectionID = activeCamera?.connectionID
    let reading = streamTask
    let opened = subscription
    streamTask = nil
    subscription = nil
    activeCamera = nil
    for pending in pendingSnapshots.values {
      pending.timeout?.cancel()
    }
    pendingSnapshots.removeAll()
    reading?.cancel()
    if let opened { try? await opened.cancel() }
    await reading?.value
    guard
      emitEvent,
      !closed,
      connectionGeneration == invalidatedGeneration,
      let disconnectedConnectionID
    else { return }
    emit(.disconnected(connectionID: disconnectedConnectionID, reason: reason))
  }

  private func active() throws -> ActiveCamera {
    guard let activeCamera else {
      throw CameraViewerError("No camera is connected")
    }
    return activeCamera
  }

  private func ensureCurrent(
    _ remote: ActiveCamera,
    operation: String
  ) throws {
    try ensureCurrent(
      remote.connectionID,
      peerID: remote.target.peerId,
      operation: operation
    )
  }

  private func ensureCurrent(
    _ connectionID: CameraViewerConnectionID,
    peerID: String,
    operation: String
  ) throws {
    guard isCurrent(connectionID, peerID: peerID) else {
      throw CameraViewerError("Cannot \(operation): camera connection changed")
    }
  }

  private func isCurrent(
    _ connectionID: CameraViewerConnectionID,
    peerID: String? = nil
  ) -> Bool {
    guard
      !closed,
      connectionGeneration == connectionID.generation,
      let activeCamera,
      activeCamera.connectionID == connectionID
    else { return false }
    return peerID == nil || activeCamera.target.peerId == peerID
  }

  private func ensureOpen(_ operation: String) throws {
    if closed { throw CameraViewerError("Cannot \(operation): viewer is closed") }
  }

  private func emit(_ event: CameraViewerEvent) {
    eventContinuation.yield(event)
  }
}

private struct LocalInfo: Encodable {
  let app: String
  let appVersion: String
  let name: String
  let sessionID: String
  let sessionClockID: String
  let sessionClockHash: String
  let sessionNowNs: Int64
  let peerID: String
  let appInstance: String

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

private struct LocalClock: Encodable {
  let peerID: String
  let sessionID: String
  let clockID: String
  let type: String
  let unit: String
  let monotonic: Bool
  let epoch: String
  let scope: String

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

private func encodeJSON<T: Encodable>(_ value: T) throws -> String {
  let encoder = JSONEncoder()
  encoder.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
  return String(decoding: try encoder.encode(value), as: UTF8.self)
}

private func utcNowNs() -> Int64 {
  Int64((Date().timeIntervalSince1970 * 1_000_000_000).rounded())
}

private func describe(_ reason: AukiStreamEndReason) -> String {
  switch reason {
  case .sourceEnded: "source ended"
  case .producerShuttingDown: "publisher stopped"
  case .sessionEnded: "session ended"
  case .producerError(let detail): "publisher error: \(detail)"
  }
}

public struct CameraViewerError: LocalizedError, Sendable {
  public let message: String

  public init(_ message: String) {
    self.message = message
  }

  public var errorDescription: String? { message }
}
