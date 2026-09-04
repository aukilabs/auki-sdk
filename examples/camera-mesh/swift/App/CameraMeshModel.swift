import AukiCameraMesh
import Combine
import Foundation
import UIKit

enum CameraMeshRole: String, CaseIterable, Identifiable {
  case viewer
  case publisher

  var id: Self { self }

  var title: String {
    switch self {
    case .viewer: "Viewer"
    case .publisher: "Publisher"
    }
  }

  var discoveryMode: AukiDiscoveryMode {
    switch self {
    case .viewer: .discoverOnly
    case .publisher: .discoverAndAdvertise
    }
  }
}

enum CameraMeshPhase: String {
  case signedOut = "Signed out"
  case authenticating = "Authenticating"
  case authenticated = "Choose a Domain"
  case starting = "Starting peer"
  case ready = "Peer ready"
  case discovering = "Discovering cameras"
  case connecting = "Connecting camera"
  case connected = "Camera connected"
  case controlling = "Sending control"
  case disconnecting = "Disconnecting camera"
  case stopping = "Stopping"
}

enum CameraTileStatus: String, Sendable {
  case connecting
  case waiting
  case live
  case awaitingApproval
  case ended
  case error
}

@MainActor
final class CameraTile: ObservableObject {
  let peerID: String
  @Published var connectionID: CameraViewerConnectionID?
  @Published var connection: CameraViewerConnection?
  @Published var status: CameraTileStatus
  @Published var message: String
  @Published var image: UIImage?
  @Published var snapshotImage: UIImage?
  @Published var snapshotHash = ""
  @Published var snapshotRelayed = false
  @Published var frameCount: UInt64 = 0
  @Published var latestSequence: UInt64?
  @Published var latestFrameAt: Date?
  @Published var paused = false
  @Published var snapshotPending = false
  @Published var controlPending = false
  @Published var switchingQuality: CameraQuality?

  init(
    peerID: String,
    connectionID: CameraViewerConnectionID? = nil,
    connection: CameraViewerConnection? = nil,
    status: CameraTileStatus,
    message: String,
    image: UIImage? = nil
  ) {
    self.peerID = peerID
    self.connectionID = connectionID
    self.connection = connection
    self.status = status
    self.message = message
    self.image = image
  }

  var name: String { connection?.name ?? "Camera \(shortCameraPeerID(peerID))" }
  var runtime: String { connection?.runtime ?? "remote" }
  var quality: CameraQuality? { connection?.quality }
  var availableQualities: [CameraQuality] { connection?.availableQualities ?? [] }
}

func isCameraApprovalRequired(_ message: String) -> Bool {
  let normalized = message.lowercased()
  return normalized.contains("approval_required")
    || normalized.contains("approval required")
}

@MainActor
final class CameraMeshModel: ObservableObject {
  @Published var email = ""
  @Published var password = ""
  @Published var selectedDomainID = ""
  @Published var selectedRole: CameraMeshRole = .viewer
  @Published var selectedCameraPeerID = ""
  @Published var remoteCard = ""
  @Published private(set) var preferredViewerQuality: CameraQuality = .medium

  @Published private(set) var domains: [AukiDomain] = []
  @Published private(set) var discoveredCameras: [CameraMeshCandidate] = []
  @Published private(set) var localCard = ""
  @Published private(set) var localPeerID = ""
  @Published private(set) var cameraTiles: [CameraTile] = []
  @Published private(set) var liveCameraCount = 0
  @Published private(set) var addingAllCameras = false
  @Published var focusedCameraPeerID: String?
  @Published private(set) var latestFrameImage: UIImage?
  @Published private(set) var snapshotImage: UIImage?
  @Published private(set) var snapshotHash = ""
  @Published private(set) var snapshotRelayed = false
  @Published private(set) var snapshotPeerID: String?
  @Published private(set) var frameCount: UInt64 = 0
  @Published private(set) var latestSequence: UInt64?
  @Published private(set) var paused = false
  @Published private(set) var pendingViewerPeerIDs: [String] = []
  @Published private(set) var approvedViewerPeerIDs: [String] = []
  @Published private(set) var lastPublisherEvent = ""
  @Published private(set) var log = ""
  @Published private(set) var phase: CameraMeshPhase = .signedOut

  private enum ConnectionAttempt {
    case discovered(CameraMeshCandidate)
    case card(String)
  }

  private var identity: AukiPeerIdentity?
  private var session: AukiSession?
  private var viewer: CameraViewer?
  private var publisher: CameraPublisher?
  private var capture: CameraCapture?
  private var provisionalPeer: AukiPeer?
  private var provisionalViewer: CameraViewer?
  private var provisionalPublisher: CameraPublisher?
  private var provisionalCapture: CameraCapture?
  private var eventTask: Task<Void, Never>?
  private var frameTask: Task<Void, Never>?
  private var retryAttempts: [String: ConnectionAttempt] = [:]
  private var automationStarted = false
  private var snapshotAfterFirstFrame = false
  private var runAcceptanceFlow = false
  private var stopAfterSnapshot = false
  private var automationAcceptanceStarted = false
  private var automationSnapshotRequested = false
  private var generation = 0

  var canLogin: Bool {
    phase == .signedOut && !email.isEmpty && !password.isEmpty
  }

  var canStart: Bool {
    phase == .authenticated && !selectedDomainID.isEmpty
  }

  var canDiscover: Bool {
    phase == .ready && selectedRole == .viewer
  }

  var canConnectDiscovered: Bool {
    selectedRole == .viewer && phase == .ready
      && cameraTiles.count < CameraMeshContract.maximumViewerConnections
      && discoveredCameras.contains(where: { $0.peerID == selectedCameraPeerID })
  }

  var canConnectCard: Bool {
    selectedRole == .viewer && phase == .ready
      && cameraTiles.count < CameraMeshContract.maximumViewerConnections
      && !remoteCard.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
  }

  var remainingCameraSlots: Int {
    max(0, CameraMeshContract.maximumViewerConnections - cameraTiles.count)
  }

  var canAddAllCameras: Bool {
    guard selectedRole == .viewer, phase == .ready, !addingAllCameras else { return false }
    if discoveredCameras.isEmpty { return true }
    return !addAllCandidates().isEmpty
  }

  var awaitingApproval: Bool {
    cameraTiles.contains { $0.status == .awaitingApproval }
  }

  var canStop: Bool {
    switch phase {
    case .authenticating, .authenticated, .starting, .ready, .discovering, .connecting,
      .connected, .controlling, .disconnecting:
      true
    default:
      false
    }
  }

  @discardableResult
  func login() async -> Bool {
    guard canLogin else { return false }
    generation += 1
    let operationGeneration = generation
    phase = .authenticating
    defer { password = "" }

    do {
      let authenticated = try await AukiSession.loginDev(email: email, password: password)
      let choices = try await authenticated.accessibleDomains()
      guard generation == operationGeneration, phase == .authenticating else { return false }
      guard !choices.isEmpty else {
        throw CameraViewerError("This User has no accessible Domains")
      }
      session = authenticated
      domains = choices
      if !choices.contains(where: { $0.id == selectedDomainID }) {
        selectedDomainID = choices[0].id
      }
      phase = .authenticated
      write("Authenticated. Choose a Domain and Camera Mesh role.")
      return true
    } catch {
      guard generation == operationGeneration else { return false }
      session = nil
      domains = []
      selectedDomainID = ""
      phase = .signedOut
      write(error)
      return false
    }
  }

  @discardableResult
  func start() async -> Bool {
    guard canStart, let session else { return false }
    generation += 1
    let operationGeneration = generation
    phase = .starting
    let role = selectedRole

    switch role {
    case .viewer:
      return await startViewer(session: session, generation: operationGeneration)
    case .publisher:
      return await startPublisher(session: session, generation: operationGeneration)
    }
  }

  private func startViewer(session: AukiSession, generation operationGeneration: Int) async
    -> Bool
  {
    var startedPeer: AukiPeer?
    var mountedViewer: CameraViewer?

    do {
      let peerIdentity = identity ?? AukiPeerIdentity.generate()
      identity = peerIdentity
      localPeerID = peerIdentity.peerId()
      write("Starting ephemeral viewer Peer ID \(localPeerID)…")

      let peer = try await session.startPeerWithDiscovery(
        domainId: selectedDomainID,
        identity: peerIdentity,
        mode: CameraMeshRole.viewer.discoveryMode
      )
      startedPeer = peer
      guard generation == operationGeneration, phase == .starting else {
        try? await peer.shutdown()
        return false
      }
      provisionalPeer = peer

      let mounted = try await CameraViewer.mount(
        peer: peer,
        displayName: ProcessInfo.processInfo.environment["AUKI_IOS_NODE_NAME"]
          ?? "iOS Camera Viewer"
      )
      mountedViewer = mounted
      provisionalPeer = nil
      guard generation == operationGeneration, phase == .starting else {
        try? await mounted.close()
        return false
      }
      provisionalViewer = mounted

      let card = try await mounted.cardJSON()
      guard generation == operationGeneration, phase == .starting else {
        try? await mounted.close()
        return false
      }

      self.session = nil
      provisionalViewer = nil
      viewer = mounted
      localPeerID = mounted.peerID
      localCard = card
      discoveredCameras = []
      selectedCameraPeerID = ""
      resetRemotePresentation(clearSnapshot: true)
      phase = .ready
      observeEvents(from: mounted, generation: operationGeneration)
      write("Viewer ready in discover-only mode. Camera publishers cannot discover it through DDS.")
      print("AUKI_IOS_CAMERA_READY PEER_CARD=\(card)")
      return true
    } catch {
      provisionalPeer = nil
      provisionalViewer = nil
      if let mountedViewer {
        try? await mountedViewer.close()
      } else if let startedPeer {
        try? await startedPeer.shutdown()
      }
      guard generation == operationGeneration else { return false }
      phase = .authenticated
      write(error)
      return false
    }
  }

  private func startPublisher(
    session: AukiSession,
    generation operationGeneration: Int
  ) async -> Bool {
    var startedPeer: AukiPeer?
    var mountedPublisher: CameraPublisher?
    let camera = CameraCapture()
    provisionalCapture = camera

    do {
      write("Requesting foreground camera access…")
      try await camera.start()
      let initialBatch = try await firstFrame(from: camera)
      guard let initialRenditions = initialBatch.initialRenditions else {
        throw CameraPublisherError("The first camera sample did not produce all three renditions")
      }
      guard generation == operationGeneration, phase == .starting else {
        await camera.stop()
        return false
      }

      latestFrameImage = UIImage(data: initialRenditions.low)
      frameCount = 1

      let peerIdentity = identity ?? AukiPeerIdentity.generate()
      identity = peerIdentity
      localPeerID = peerIdentity.peerId()
      write("Starting ephemeral publisher Peer ID \(localPeerID)…")

      let peer = try await session.startPeerWithDiscovery(
        domainId: selectedDomainID,
        identity: peerIdentity,
        mode: CameraMeshRole.publisher.discoveryMode
      )
      startedPeer = peer
      guard generation == operationGeneration, phase == .starting else {
        try? await peer.shutdown()
        await camera.stop()
        return false
      }
      provisionalPeer = peer

      let mounted = try await CameraPublisher.mount(
        peer: peer,
        displayName: ProcessInfo.processInfo.environment["AUKI_IOS_CAMERA_NAME"]
          ?? "iPhone Camera",
        initialRenditions: initialRenditions
      )
      mountedPublisher = mounted
      provisionalPeer = nil
      provisionalPublisher = mounted
      guard generation == operationGeneration, phase == .starting else {
        try? await mounted.close()
        await camera.stop()
        return false
      }
      let initialPaused = try await mounted.paused()
      guard generation == operationGeneration, phase == .starting else {
        try? await mounted.close()
        await camera.stop()
        return false
      }

      self.session = nil
      provisionalPublisher = nil
      provisionalCapture = nil
      publisher = mounted
      capture = camera
      localPeerID = mounted.peerID
      localCard = mounted.cardJSON
      pendingViewerPeerIDs = []
      approvedViewerPeerIDs = []
      lastPublisherEvent = "Waiting for a viewer"
      paused = initialPaused
      phase = .ready
      observeCapturedFrames(from: camera, publisher: mounted, generation: operationGeneration)
      observePublisherEvents(from: mounted, generation: operationGeneration)
      write("Publisher ready and discoverable. Approve each exact viewer Peer ID before access.")
      print("AUKI_IOS_CAMERA_PUBLISHER_READY PEER_CARD=\(mounted.cardJSON)")
      return true
    } catch {
      provisionalPeer = nil
      provisionalPublisher = nil
      provisionalCapture = nil
      if let mountedPublisher {
        try? await mountedPublisher.close()
      } else if let startedPeer {
        try? await startedPeer.shutdown()
      }
      await camera.stop()
      guard generation == operationGeneration else { return false }
      resetRemotePresentation(clearSnapshot: true)
      resetPublisherPresentation()
      phase = .authenticated
      write(error)
      return false
    }
  }

  func approveViewer(_ peerID: String) async {
    guard phase == .ready, let publisher else { return }
    let operationGeneration = generation
    do {
      try await publisher.approve(peerID: peerID)
      guard
        generation == operationGeneration,
        self.publisher === publisher,
        phase == .ready
      else { return }
      let pending = try await publisher.pendingApprovals()
      guard
        generation == operationGeneration,
        self.publisher === publisher,
        phase == .ready
      else { return }
      pendingViewerPeerIDs = pending.sorted()
      if !approvedViewerPeerIDs.contains(peerID) {
        approvedViewerPeerIDs.append(peerID)
        approvedViewerPeerIDs.sort()
      }
      lastPublisherEvent = "Approved viewer \(peerID)"
      write("Approved exact viewer Peer ID \(peerID). The viewer may retry now.")
      print("AUKI_IOS_CAMERA_VIEWER_APPROVED peer=\(peerID)")
    } catch {
      guard
        generation == operationGeneration,
        self.publisher === publisher,
        phase == .ready
      else { return }
      write(error)
    }
  }

  func revokeViewer(_ peerID: String) async {
    guard phase == .ready, let publisher else { return }
    let operationGeneration = generation
    do {
      try await publisher.revoke(peerID: peerID)
      guard
        generation == operationGeneration,
        self.publisher === publisher,
        phase == .ready
      else { return }
      let pending = try await publisher.pendingApprovals()
      guard
        generation == operationGeneration,
        self.publisher === publisher,
        phase == .ready
      else { return }
      pendingViewerPeerIDs = pending.sorted()
      approvedViewerPeerIDs.removeAll(where: { $0 == peerID })
      lastPublisherEvent = "Revoked viewer \(peerID)"
      write("Revoked viewer Peer ID \(peerID).")
      print("AUKI_IOS_CAMERA_VIEWER_REVOKED peer=\(peerID)")
    } catch {
      guard
        generation == operationGeneration,
        self.publisher === publisher,
        phase == .ready
      else { return }
      write(error)
    }
  }

  @discardableResult
  func discover() async -> Bool {
    guard canDiscover, let viewer else { return false }
    let operationGeneration = generation
    phase = .discovering

    do {
      let candidates = try await viewer.discoverCameras()
      guard generation == operationGeneration, self.viewer === viewer else { return false }
      discoveredCameras = candidates
      if !candidates.contains(where: { $0.peerID == selectedCameraPeerID }) {
        selectedCameraPeerID = candidates.first?.peerID ?? ""
      }
      phase = .ready
      write("Discovery returned \(candidates.count) Stream publisher(s).")
      write(
        "Candidates remain untrusted until the exact connection authenticates their Peer ID and Domain."
      )
      return true
    } catch {
      guard generation == operationGeneration, self.viewer === viewer else { return false }
      phase = .ready
      write(error)
      return false
    }
  }

  @discardableResult
  func connectSelectedCamera() async -> Bool {
    guard
      canConnectDiscovered,
      let candidate = discoveredCameras.first(where: { $0.peerID == selectedCameraPeerID })
    else { return false }
    return await connect(using: .discovered(candidate), peerID: candidate.peerID)
  }

  @discardableResult
  func connectPastedCard() async -> Bool {
    guard canConnectCard else { return false }
    let card = remoteCard.trimmingCharacters(in: .whitespacesAndNewlines)
    do {
      let target = try nativeCameraTarget(cardJSON: card, domainID: selectedDomainID)
      return await connect(using: .card(card), peerID: target.peerId)
    } catch {
      write(error)
      return false
    }
  }

  @discardableResult
  func retryCamera(peerID: String) async -> Bool {
    guard phase == .ready, let attempt = retryAttempts[peerID] else { return false }
    return await connect(using: attempt, peerID: peerID)
  }

  @discardableResult
  func pauseCamera(peerID: String) async -> Bool {
    guard phase == .ready, let viewer,
      let connectionID = tile(peerID: peerID)?.connectionID
    else { return false }
    let operationGeneration = generation
    updateTile(peerID: peerID) { $0.controlPending = true }
    do {
      try await viewer.pause(connectionID: connectionID)
      guard generation == operationGeneration, self.viewer === viewer else { return false }
      updateTile(peerID: peerID) {
        guard $0.connectionID == connectionID else { return }
        $0.paused = true
        $0.controlPending = false
        $0.message = "Source paused for every viewer"
      }
      return true
    } catch {
      guard generation == operationGeneration, self.viewer === viewer else { return false }
      updateTile(peerID: peerID) { $0.controlPending = false }
      write(error)
      return false
    }
  }

  @discardableResult
  func resumeCamera(peerID: String) async -> Bool {
    guard phase == .ready, let viewer,
      let connectionID = tile(peerID: peerID)?.connectionID
    else { return false }
    let operationGeneration = generation
    updateTile(peerID: peerID) { $0.controlPending = true }
    do {
      try await viewer.resume(connectionID: connectionID)
      guard generation == operationGeneration, self.viewer === viewer else { return false }
      updateTile(peerID: peerID) {
        guard $0.connectionID == connectionID else { return }
        $0.paused = false
        $0.controlPending = false
        $0.message = "Live feed"
      }
      return true
    } catch {
      guard generation == operationGeneration, self.viewer === viewer else { return false }
      updateTile(peerID: peerID) { $0.controlPending = false }
      write(error)
      return false
    }
  }

  @discardableResult
  func requestSnapshot(peerID: String) async -> Bool {
    guard phase == .ready, let viewer,
      let connectionID = tile(peerID: peerID)?.connectionID,
      tile(peerID: peerID)?.snapshotPending == false
    else { return false }
    let operationGeneration = generation
    updateTile(peerID: peerID) { $0.snapshotPending = true }
    do {
      _ = try await viewer.requestSnapshot(connectionID: connectionID)
      guard generation == operationGeneration, self.viewer === viewer else { return false }
      return true
    } catch {
      guard generation == operationGeneration, self.viewer === viewer else { return false }
      updateTile(peerID: peerID) { $0.snapshotPending = false }
      write(error)
      return false
    }
  }

  func removeCamera(peerID: String) async {
    guard selectedRole == .viewer else { return }
    let operationGeneration = generation
    let connectionID = tile(peerID: peerID)?.connectionID
    cameraTiles.removeAll { $0.peerID == peerID }
    refreshLiveCameraCount()
    retryAttempts.removeValue(forKey: peerID)
    selectCameraAfterRemoval(peerID)
    guard let viewer, let connectionID else { return }
    await viewer.disconnect(connectionID: connectionID, reason: "Removed by operator")
    guard generation == operationGeneration, self.viewer === viewer else { return }
    write("Removed camera \(peerID).")
  }

  func discoverAndAddAllCameras() async {
    guard phase == .ready, selectedRole == .viewer, !addingAllCameras else { return }

    if discoveredCameras.isEmpty {
      guard await discover() else { return }
    }

    let candidates = addAllCandidates()
    guard !candidates.isEmpty else {
      write("Every discovered camera is already on the wall.")
      return
    }

    addingAllCameras = true
    defer { addingAllCameras = false }
    let quality = preferredViewerQuality
    let tasks = candidates.map { candidate in
      Task { [weak self] in
        guard let self else { return false }
        return await self.connect(
          using: .discovered(candidate),
          peerID: candidate.peerID,
          preferredQuality: quality
        )
      }
    }
    for task in tasks { _ = await task.value }
  }

  private func addAllCandidates() -> [CameraMeshCandidate] {
    var newCameraSlots = remainingCameraSlots
    return discoveredCameras.filter { candidate in
      guard let existing = tile(peerID: candidate.peerID) else {
        guard newCameraSlots > 0 else { return false }
        newCameraSlots -= 1
        return true
      }
      switch existing.status {
      case .awaitingApproval, .ended, .error:
        return true
      case .connecting, .waiting, .live:
        return false
      }
    }
  }

  func focusCamera(peerID: String) {
    guard cameraTiles.contains(where: { $0.peerID == peerID }) else { return }
    focusedCameraPeerID = peerID
  }

  func setPreferredViewerQuality(forColumnCount columns: Int) {
    preferredViewerQuality = preferredCameraQuality(forColumnCount: columns)
  }

  @discardableResult
  func switchCameraQuality(peerID: String, quality: CameraQuality) async -> Bool {
    guard
      phase == .ready,
      let viewer,
      let tile = tile(peerID: peerID),
      let connectionID = tile.connectionID,
      tile.quality != quality,
      tile.availableQualities.contains(quality),
      tile.switchingQuality == nil,
      !tile.snapshotPending
    else { return false }

    let operationGeneration = generation
    updateTile(peerID: peerID) {
      $0.switchingQuality = quality
      $0.message = "Opening the \(quality.title) rendition…"
    }
    do {
      let replacementID = try await viewer.switchQuality(
        connectionID: connectionID,
        quality: quality
      )
      guard generation == operationGeneration, self.viewer === viewer else { return false }
      updateTile(peerID: peerID) {
        $0.connectionID = replacementID
        $0.switchingQuality = nil
      }
      write("Switched \(peerID) to \(CameraMeshContract.profile(quality).label).")
      return true
    } catch {
      guard generation == operationGeneration, self.viewer === viewer else { return false }
      updateTile(peerID: peerID) {
        $0.switchingQuality = nil
        $0.message = error.localizedDescription
      }
      write(error)
      return false
    }
  }

  func moveFocus(by offset: Int) {
    guard !cameraTiles.isEmpty else {
      focusedCameraPeerID = nil
      return
    }
    let current = cameraTiles.firstIndex { $0.peerID == focusedCameraPeerID } ?? 0
    let next = (current + offset + cameraTiles.count) % cameraTiles.count
    focusedCameraPeerID = cameraTiles[next].peerID
  }

  func stop(reason: String = "Camera Mesh stopped") async {
    guard canStop else { return }
    phase = .stopping
    generation += 1

    let mountedViewer = viewer ?? provisionalViewer
    let mountedPublisher = publisher ?? provisionalPublisher
    let peer = mountedViewer == nil && mountedPublisher == nil ? provisionalPeer : nil
    let activeCapture = capture ?? provisionalCapture
    let observing = eventTask
    let forwardingFrames = frameTask
    viewer = nil
    publisher = nil
    capture = nil
    provisionalViewer = nil
    provisionalPublisher = nil
    provisionalCapture = nil
    provisionalPeer = nil
    eventTask = nil
    frameTask = nil

    do {
      await activeCapture?.stop()
      forwardingFrames?.cancel()
      await forwardingFrames?.value

      if let mountedViewer {
        try await mountedViewer.close()
      } else if let mountedPublisher {
        try await mountedPublisher.close()
      } else if let peer {
        try await peer.shutdown()
      }
      observing?.cancel()
      await observing?.value
      write(reason)
    } catch {
      observing?.cancel()
      await observing?.value
      write(error)
    }

    session = nil
    domains = []
    selectedDomainID = ""
    discoveredCameras = []
    selectedCameraPeerID = ""
    localCard = ""
    localPeerID = ""
    resetRemotePresentation(clearSnapshot: true)
    resetPublisherPresentation()
    phase = .signedOut
    print("AUKI_IOS_CAMERA_STOPPED")
  }

  func runAutomationIfConfigured() {
    guard !automationStarted else { return }
    automationStarted = true
    let environment = ProcessInfo.processInfo.environment
    guard
      let automationEmail = environment["AUKI_IOS_EMAIL"],
      let automationPassword = environment["AUKI_IOS_PASSWORD"],
      let automationDomain = environment["AUKI_IOS_DOMAIN_ID"]
    else { return }

    email = automationEmail
    password = automationPassword
    selectedRole = .viewer
    selectedDomainID = automationDomain
    remoteCard = environment["AUKI_IOS_REMOTE_CARD"] ?? ""
    snapshotAfterFirstFrame = environment["AUKI_IOS_SNAPSHOT_AFTER_FIRST_FRAME"] == "1"
    runAcceptanceFlow = environment["AUKI_IOS_RUN_ACCEPTANCE"] == "1"
    stopAfterSnapshot = environment["AUKI_IOS_STOP_AFTER_SNAPSHOT"] == "1"

    Task { [weak self] in
      guard let self, await self.login() else { return }
      self.selectedDomainID = automationDomain
      guard self.domains.contains(where: { $0.id == automationDomain }) else {
        self.write("Configured automation Domain is not accessible.")
        return
      }
      guard await self.start() else { return }
      guard !self.remoteCard.isEmpty else { return }
      let connected = await self.connectPastedCard()
      guard
        !connected,
        self.awaitingApproval,
        let delayText = environment["AUKI_IOS_RETRY_AFTER_APPROVAL_SECONDS"],
        let delay = Double(delayText),
        delay > 0
      else { return }
      try? await Task.sleep(for: .seconds(delay))
      guard !Task.isCancelled else { return }
      if let peerID = self.cameraTiles.first(where: { $0.status == .awaitingApproval })?.peerID {
        _ = await self.retryCamera(peerID: peerID)
      }
    }
  }

  private func connect(
    using attempt: ConnectionAttempt,
    peerID: String,
    preferredQuality: CameraQuality? = nil
  ) async -> Bool {
    guard phase == .ready, let viewer else { return false }
    if let existing = tile(peerID: peerID),
      existing.status == .connecting || existing.status == .waiting || existing.status == .live
    {
      return existing.status == .live
    }
    guard tile(peerID: peerID) != nil || remainingCameraSlots > 0 else {
      write("Camera wall is full (\(CameraMeshContract.maximumViewerConnections) cameras).")
      return false
    }

    let operationGeneration = generation
    let quality = preferredQuality ?? preferredViewerQuality
    retryAttempts[peerID] = attempt
    if tile(peerID: peerID) == nil {
      cameraTiles.append(
        CameraTile(
          peerID: peerID,
          status: .connecting,
          message: "Authenticating camera…"
        ))
      if focusedCameraPeerID == nil { focusedCameraPeerID = peerID }
    } else {
      updateTile(peerID: peerID) {
        $0.connectionID = nil
        $0.connection = nil
        $0.status = .connecting
        $0.message = "Authenticating camera…"
        $0.paused = false
        $0.snapshotPending = false
        $0.controlPending = false
        $0.switchingQuality = nil
      }
    }

    do {
      let connectionID: CameraViewerConnectionID
      switch attempt {
      case .discovered(let candidate):
        connectionID = try await viewer.connect(
          candidate: candidate,
          preferredQuality: quality
        )
      case .card(let card):
        connectionID = try await viewer.connect(
          cardJSON: card,
          preferredQuality: quality
        )
      }
      guard generation == operationGeneration, self.viewer === viewer,
        tile(peerID: peerID) != nil
      else {
        await viewer.disconnect(connectionID: connectionID, reason: "Connection superseded")
        return false
      }
      updateTile(peerID: peerID) {
        $0.connectionID = connectionID
        if $0.connection == nil {
          $0.status = .waiting
          $0.message = "Waiting for the first frame…"
        }
      }
      write("Camera \(peerID) authenticated. Waiting for JPEG frames.")
      return true
    } catch {
      guard generation == operationGeneration, self.viewer === viewer else { return false }
      let message = error.localizedDescription
      let approvalRequired = isCameraApprovalRequired(message)
      updateTile(peerID: peerID) {
        $0.connectionID = nil
        $0.connection = nil
        $0.status = approvalRequired ? .awaitingApproval : .error
        $0.message =
          approvalRequired
          ? "Approve this viewer on the publisher, then retry."
          : message
        $0.snapshotPending = false
        $0.controlPending = false
      }
      write(error)
      if approvalRequired {
        write("The publisher received this viewer Peer ID. Approve it there, then select Retry.")
        print("AUKI_IOS_CAMERA_APPROVAL_REQUIRED VIEWER_PEER=\(localPeerID)")
      }
      return false
    }
  }

  private func firstFrame(from source: CameraCapture) async throws -> CameraCaptureBatch {
    try await withThrowingTaskGroup(of: CameraCaptureBatch.self) { group in
      group.addTask {
        var frames = source.frames.makeAsyncIterator()
        guard let batch = try await frames.next() else {
          throw CameraPublisherError("Camera capture stopped before its first frame")
        }
        return batch
      }
      group.addTask {
        try await Task.sleep(for: .seconds(10))
        throw CameraPublisherError("Camera did not produce a JPEG within 10 seconds")
      }

      defer { group.cancelAll() }
      guard let batch = try await group.next() else {
        throw CameraPublisherError("Camera did not produce its first rendition batch")
      }
      return batch
    }
  }

  private func observeCapturedFrames(
    from source: CameraCapture,
    publisher target: CameraPublisher,
    generation frameGeneration: Int
  ) {
    let frames = source.frames
    frameTask = Task { [weak self] in
      do {
        for try await batch in frames {
          guard !Task.isCancelled else { return }
          do {
            for quality in CameraQuality.allCases {
              if let jpeg = batch[quality] {
                try await target.updateLatestJPEG(jpeg, quality: quality)
              }
            }
          } catch {
            guard
              let self,
              self.generation == frameGeneration,
              self.publisher === target,
              self.phase == .ready
            else { return }
            self.publisherFrameForwardingFailed(
              error,
              generation: frameGeneration,
              publisher: target
            )
            return
          }

          guard
            let self,
            self.generation == frameGeneration,
            self.publisher === target,
            self.phase == .ready
          else { return }
          if let previewJPEG = batch[.low] {
            self.latestFrameImage = UIImage(data: previewJPEG)
            self.frameCount &+= 1
          }
        }
      } catch {
        guard
          let self,
          self.generation == frameGeneration,
          self.publisher === target,
          self.phase == .ready
        else { return }
        self.publisherCaptureFailed(error, generation: frameGeneration, publisher: target)
        return
      }

      guard
        let self,
        !Task.isCancelled,
        self.generation == frameGeneration,
        self.publisher === target,
        self.phase == .ready
      else { return }
      self.publisherCaptureFailed(
        CameraPublisherError("Camera capture ended unexpectedly"),
        generation: frameGeneration,
        publisher: target
      )
    }
  }

  private func publisherCaptureFailed(
    _ error: Error,
    generation frameGeneration: Int,
    publisher source: CameraPublisher
  ) {
    guard generation == frameGeneration, publisher === source, phase == .ready else { return }
    lastPublisherEvent = "Camera capture failed"
    write(error)
    print("AUKI_IOS_CAMERA_CAPTURE_FAILURE \(error.localizedDescription)")
    stopPublisherAfterFrameFailure(reason: "Camera capture stopped; publisher unadvertised")
  }

  private func publisherFrameForwardingFailed(
    _ error: Error,
    generation frameGeneration: Int,
    publisher source: CameraPublisher
  ) {
    guard generation == frameGeneration, publisher === source, phase == .ready else { return }
    lastPublisherEvent = "Camera frame forwarding failed"
    write(error)
    print("AUKI_IOS_CAMERA_FRAME_FORWARDING_FAILURE \(error.localizedDescription)")
    stopPublisherAfterFrameFailure(
      reason: "Camera frame forwarding stopped; publisher unadvertised")
  }

  private func stopPublisherAfterFrameFailure(reason: String) {
    // The stop task must not await the frame task that is asking it to stop.
    frameTask = nil
    Task { [weak self] in
      await self?.stop(reason: reason)
    }
  }

  private func observePublisherEvents(
    from source: CameraPublisher,
    generation eventGeneration: Int
  ) {
    eventTask = Task { [weak self] in
      while !Task.isCancelled {
        do {
          guard let event = try await source.nextEvent() else { return }
          guard !Task.isCancelled else { return }
          await self?.handlePublisher(event, from: source, generation: eventGeneration)
        } catch {
          guard
            let self,
            self.generation == eventGeneration,
            self.publisher === source,
            self.phase != .stopping
          else { return }
          self.lastPublisherEvent = "Publisher event loop failed"
          self.write(error)
          return
        }
      }
    }
  }

  private func handlePublisher(
    _ event: CameraPublisherEvent,
    from source: CameraPublisher,
    generation eventGeneration: Int
  ) async {
    guard generation == eventGeneration, publisher === source, phase == .ready else { return }

    switch event {
    case .approvalRequired(let peerID):
      if !approvedViewerPeerIDs.contains(peerID), !pendingViewerPeerIDs.contains(peerID) {
        pendingViewerPeerIDs.append(peerID)
        pendingViewerPeerIDs.sort()
      }
      lastPublisherEvent = "Viewer approval required"
      write("Viewer requests access. Verify and approve exact Peer ID \(peerID).")
      print("AUKI_IOS_CAMERA_APPROVAL_PENDING peer=\(peerID)")

    case .controlReceived(let peerID, let control):
      let currentPaused = try? await source.paused()
      guard generation == eventGeneration, publisher === source, phase == .ready else { return }
      if let currentPaused {
        paused = currentPaused
      }
      lastPublisherEvent = "\(control) from \(peerID)"
      write("Accepted \(control) from approved viewer \(peerID).")
      print("AUKI_IOS_CAMERA_CONTROL peer=\(peerID) control=\(control)")

    case .snapshotStaged(let peerID, let requestID, let sha256, let size):
      lastPublisherEvent = "Snapshot \(requestID) staged"
      write("Staged \(size)-byte snapshot for \(peerID); SHA-256 \(sha256).")
      print(
        "AUKI_IOS_CAMERA_SNAPSHOT_STAGED peer=\(peerID) request=\(requestID) "
          + "sha256=\(sha256) bytes=\(size)"
      )

    case .failed(let message):
      lastPublisherEvent = "Publisher runtime error"
      write(message)
      print("AUKI_IOS_CAMERA_PUBLISHER_FAILURE \(message)")
    }

    await reconcilePendingApprovals(from: source, generation: eventGeneration)
  }

  private func reconcilePendingApprovals(
    from source: CameraPublisher,
    generation eventGeneration: Int
  ) async {
    do {
      let pending = try await source.pendingApprovals()
      guard generation == eventGeneration, publisher === source, phase == .ready else { return }
      pendingViewerPeerIDs = pending.sorted()
    } catch {
      guard generation == eventGeneration, publisher === source, phase == .ready else { return }
      lastPublisherEvent = "Could not refresh pending viewers"
      write(error)
    }
  }

  private func observeEvents(from source: CameraViewer, generation eventGeneration: Int) {
    let events = source.events
    eventTask = Task { [weak self] in
      for await event in events {
        guard !Task.isCancelled else { return }
        self?.handle(event, from: source, generation: eventGeneration)
      }
    }
  }

  private func handle(
    _ event: CameraViewerEvent,
    from source: CameraViewer,
    generation eventGeneration: Int
  ) {
    guard generation == eventGeneration, viewer === source else { return }

    switch event {
    case .status(let connectionID, let message):
      if let connectionID {
        updateTile(connectionID: connectionID) { $0.message = message }
      }
      write(message)

    case .connected(let connectionID, let connected):
      guard phase == .ready else { return }
      updateTile(peerID: connected.peerID) {
        $0.connectionID = connectionID
        $0.connection = connected
        $0.status = .waiting
        $0.message = "Waiting for the first \(connected.quality.title) frame…"
        $0.paused = false
        $0.switchingQuality = nil
      }
      write(
        "Connected to \(connected.name) (\(connected.runtime)) at \(connected.quality.title)."
      )
      print(
        "AUKI_IOS_CAMERA_CONNECTED peer=\(connected.peerID) runtime=\(connected.runtime)"
      )

    case .frame(let connectionID, let frame):
      guard let peerID = tile(connectionID: connectionID)?.peerID else { return }
      guard let image = UIImage(data: frame.jpeg) else {
        write("UIKit could not decode camera JPEG sequence \(frame.sequence).")
        return
      }
      let firstFrame = tile(peerID: peerID)?.frameCount == 0
      updateTile(peerID: peerID) {
        $0.image = image
        $0.frameCount &+= 1
        $0.latestSequence = frame.sequence
        $0.latestFrameAt = Date()
        $0.status = .live
        $0.message = $0.paused ? "Source paused" : "Live feed"
      }
      latestFrameImage = image
      frameCount &+= 1
      latestSequence = frame.sequence
      if firstFrame {
        print(
          "AUKI_IOS_CAMERA_FRAME peer=\(peerID) "
            + "sequence=\(frame.sequence) bytes=\(frame.jpeg.count)"
        )
      }
      let cameraFrameCount = tile(peerID: peerID)?.frameCount ?? 0
      if runAcceptanceFlow, cameraFrameCount >= 2, !automationAcceptanceStarted {
        automationAcceptanceStarted = true
        Task { [weak self] in await self?.runAutomatedAcceptanceFlow(peerID: peerID) }
      } else if snapshotAfterFirstFrame && !automationSnapshotRequested {
        automationSnapshotRequested = true
        Task { [weak self] in _ = await self?.requestSnapshot(peerID: peerID) }
      }

    case .snapshot(let connectionID, let snapshot):
      guard let peerID = tile(connectionID: connectionID)?.peerID else { return }
      let image = UIImage(data: snapshot.jpeg)
      updateTile(peerID: peerID) {
        $0.snapshotPending = false
        $0.snapshotHash = snapshot.sha256
        $0.snapshotRelayed = snapshot.relayed
        $0.snapshotImage = image
      }
      snapshotHash = snapshot.sha256
      snapshotRelayed = snapshot.relayed
      snapshotPeerID = peerID
      if let image {
        snapshotImage = image
      } else {
        write("UIKit could not decode the verified snapshot JPEG.")
      }
      write("Snapshot \(snapshot.requestID) received and SHA-256 verified.")
      print(
        "AUKI_IOS_CAMERA_SNAPSHOT request=\(snapshot.requestID) "
          + "sha256=\(snapshot.sha256) bytes=\(snapshot.jpeg.count)"
      )
      if stopAfterSnapshot {
        Task { [weak self] in await self?.stop(reason: "Automation completed") }
      }

    case .disconnected(let connectionID, let reason):
      guard let peerID = tile(connectionID: connectionID)?.peerID else { return }
      updateTile(peerID: peerID) {
        $0.connectionID = nil
        $0.connection = nil
        $0.status = .ended
        $0.message = reason
        $0.paused = false
        $0.snapshotPending = false
        $0.controlPending = false
        $0.switchingQuality = nil
      }
      write(reason)
      print("AUKI_IOS_CAMERA_DISCONNECTED peer=\(peerID)")

    case .failed(let connectionID, let reason):
      write(reason)
      if let connectionID, let peerID = tile(connectionID: connectionID)?.peerID {
        updateTile(peerID: peerID) {
          if reason.localizedCaseInsensitiveContains("snapshot") {
            $0.snapshotPending = false
          } else {
            $0.connectionID = nil
            $0.connection = nil
            $0.status = .error
            $0.paused = false
            $0.controlPending = false
            $0.switchingQuality = nil
          }
          $0.message = reason
        }
      }
      print("AUKI_IOS_CAMERA_FAILURE \(reason)")
    }
  }

  private func resetRemotePresentation(clearSnapshot: Bool = false) {
    cameraTiles = []
    liveCameraCount = 0
    retryAttempts = [:]
    focusedCameraPeerID = nil
    addingAllCameras = false
    latestFrameImage = nil
    frameCount = 0
    latestSequence = nil
    paused = false
    automationSnapshotRequested = false
    automationAcceptanceStarted = false
    if clearSnapshot {
      snapshotImage = nil
      snapshotHash = ""
      snapshotRelayed = false
      snapshotPeerID = nil
    }
  }

  private func tile(peerID: String) -> CameraTile? {
    cameraTiles.first { $0.peerID == peerID }
  }

  private func tile(connectionID: CameraViewerConnectionID) -> CameraTile? {
    cameraTiles.first { $0.connectionID == connectionID }
  }

  private func updateTile(peerID: String, _ update: (CameraTile) -> Void) {
    guard let tile = cameraTiles.first(where: { $0.peerID == peerID }) else { return }
    let wasLive = tile.status == .live
    update(tile)
    if wasLive != (tile.status == .live) { refreshLiveCameraCount() }
  }

  private func updateTile(
    connectionID: CameraViewerConnectionID,
    _ update: (CameraTile) -> Void
  ) {
    guard let tile = cameraTiles.first(where: { $0.connectionID == connectionID }) else {
      return
    }
    let wasLive = tile.status == .live
    update(tile)
    if wasLive != (tile.status == .live) { refreshLiveCameraCount() }
  }

  private func refreshLiveCameraCount() {
    liveCameraCount = cameraTiles.filter { $0.status == .live }.count
  }

  private func selectCameraAfterRemoval(_ removedPeerID: String) {
    guard focusedCameraPeerID == removedPeerID else { return }
    focusedCameraPeerID = cameraTiles.first?.peerID
  }

  private func resetPublisherPresentation() {
    pendingViewerPeerIDs = []
    approvedViewerPeerIDs = []
    lastPublisherEvent = ""
  }

  private func write(_ value: some StringProtocol) {
    log = String("\(log)\(value)\n".suffix(12_288))
  }

  private func write(_ error: Error) {
    write(error.localizedDescription)
  }

  private func runAutomatedAcceptanceFlow(peerID: String) async {
    guard await pauseCamera(peerID: peerID) else { return }
    try? await Task.sleep(for: .milliseconds(500))
    guard !Task.isCancelled, await resumeCamera(peerID: peerID) else { return }
    automationSnapshotRequested = true
    _ = await requestSnapshot(peerID: peerID)
  }
}

func shortCameraPeerID(_ peerID: String) -> String {
  guard peerID.count > 20 else { return peerID }
  return "\(peerID.prefix(10))…\(peerID.suffix(8))"
}

func preferredCameraQuality(forColumnCount columns: Int) -> CameraQuality {
  switch columns {
  case ...1: .high
  case 2: .medium
  default: .low
  }
}
