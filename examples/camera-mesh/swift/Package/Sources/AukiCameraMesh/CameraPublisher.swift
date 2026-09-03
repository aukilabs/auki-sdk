import Foundation

public enum CameraPublisherEvent: Equatable, Sendable {
  case approvalRequired(peerID: String)
  case controlReceived(peerID: String, control: String)
  case snapshotStaged(
    peerID: String,
    requestID: String,
    sha256: String,
    size: UInt64
  )
  case failed(message: String)
}

/// Foreground-only Camera Mesh publisher backed by the shared Rust application.
///
/// Camera Mesh metadata, authenticated viewer approval, protocol serving,
/// control handling, snapshot replies, and ordered shutdown remain in Rust.
/// This actor only presents that application as a small Swift lifecycle.
public actor CameraPublisher {
  public nonisolated let peer: AukiPeer
  public nonisolated let peerID: String
  public nonisolated let domainID: String
  public nonisolated let card: AukiPeerCard
  public nonisolated let cardJSON: String
  public nonisolated let protocolIDs: [String]

  private let publisher: AukiCameraPublisher
  private var closeTask: Task<Void, any Error>?
  private var eventWaitInFlight = false
  private var closed = false

  private init(
    peer: AukiPeer,
    publisher: AukiCameraPublisher,
    card: AukiPeerCard,
    cardJSON: String
  ) {
    self.peer = peer
    self.publisher = publisher
    self.card = card
    self.cardJSON = cardJSON
    peerID = card.peerId
    domainID = card.domainId
    protocolIDs = card.protocols
  }

  public static func mount(
    peer: AukiPeer,
    displayName: String = "iOS Camera",
    initialJPEG: Data
  ) async throws -> CameraPublisher {
    let publisher = try await AukiCameraPublisher.mount(
      peer: peer,
      displayName: displayName,
      initialJpeg: initialJPEG
    )

    do {
      let cardJSON = try publisher.cardJson()
      let card = try peerCardFromJson(json: cardJSON)
      return CameraPublisher(
        peer: peer,
        publisher: publisher,
        card: card,
        cardJSON: cardJSON
      )
    } catch {
      try? await publisher.close()
      throw error
    }
  }

  /// Atomically replace the JPEG used by future Stream frames and snapshots.
  /// Rust retains only the latest frame.
  public func updateLatestJPEG(_ jpeg: Data) throws {
    try ensureOpen("update the latest camera frame")
    try publisher.updateFrame(jpeg: jpeg)
  }

  /// Receive one publisher event, or `nil` when the publisher is closed.
  ///
  /// Consume events from one task. Rust owns the bounded event queue so this
  /// facade does not introduce another buffer that could discard approvals.
  public func nextEvent() async throws -> CameraPublisherEvent? {
    if closed { return nil }
    guard !eventWaitInFlight else {
      throw CameraPublisherError(
        "Cannot receive publisher events from more than one task"
      )
    }

    eventWaitInFlight = true
    defer { eventWaitInFlight = false }

    let event: AukiCameraPublisherEvent?
    do {
      event = try await publisher.nextEvent()
    } catch {
      if closed { return nil }
      throw error
    }
    guard !closed, let event else { return nil }
    return try translate(event)
  }

  public func approve(peerID: String) throws {
    try ensureOpen("approve a camera viewer")
    try publisher.approve(peerId: peerID)
  }

  public func revoke(peerID: String) throws {
    try ensureOpen("revoke a camera viewer")
    try publisher.revoke(peerId: peerID)
  }

  public func pendingApprovals() throws -> [String] {
    try ensureOpen("list pending camera viewers")
    return try publisher.pendingApprovals()
  }

  public func paused() throws -> Bool {
    try ensureOpen("read the camera pause state")
    return try publisher.paused()
  }

  /// Replay-safe shutdown. Every concurrent caller waits for the same Rust
  /// cleanup barrier, which closes protocol endpoints before the peer.
  public func close() async throws {
    if let closeTask {
      return try await closeTask.value
    }

    closed = true
    let publisher = self.publisher
    let task = Task {
      try await publisher.close()
    }
    closeTask = task
    try await task.value
  }

  private func ensureOpen(_ operation: String) throws {
    if closed {
      throw CameraPublisherError("Cannot \(operation): publisher is closed")
    }
  }
}

private func translate(
  _ event: AukiCameraPublisherEvent
) throws -> CameraPublisherEvent {
  switch event.kind {
  case .approvalRequired:
    guard let peerID = event.peerId else {
      throw CameraPublisherError("Approval event is missing its viewer Peer ID")
    }
    return .approvalRequired(peerID: peerID)

  case .controlReceived:
    guard let peerID = event.peerId, let control = event.control else {
      throw CameraPublisherError("Control event is missing its viewer or control")
    }
    return .controlReceived(peerID: peerID, control: control)

  case .snapshotStaged:
    guard
      let peerID = event.peerId,
      let requestID = event.requestId,
      let sha256 = event.sha256,
      let size = event.size
    else {
      throw CameraPublisherError("Snapshot event is missing required fields")
    }
    return .snapshotStaged(
      peerID: peerID,
      requestID: requestID,
      sha256: sha256,
      size: size
    )

  case .runtimeError:
    return .failed(message: event.error ?? "Camera publisher runtime failed")
  }
}

public struct CameraPublisherError: LocalizedError, Sendable {
  public let message: String

  public init(_ message: String) {
    self.message = message
  }

  public var errorDescription: String? { message }
}
