import AukiCameraMesh
import Combine
import Foundation
import UIKit

enum CameraMeshPhase: String {
  case signedOut = "Signed out"
  case authenticating = "Authenticating"
  case authenticated = "Choose a Domain"
  case starting = "Starting viewer"
  case ready = "Viewer ready"
  case discovering = "Discovering cameras"
  case connecting = "Connecting camera"
  case connected = "Camera connected"
  case controlling = "Sending control"
  case disconnecting = "Disconnecting camera"
  case stopping = "Stopping"
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
  @Published var selectedCameraPeerID = ""
  @Published var remoteCard = ""

  @Published private(set) var domains: [AukiDomain] = []
  @Published private(set) var discoveredCameras: [CameraMeshCandidate] = []
  @Published private(set) var localCard = ""
  @Published private(set) var localPeerID = ""
  @Published private(set) var connection: CameraViewerConnection?
  @Published private(set) var latestFrameImage: UIImage?
  @Published private(set) var snapshotImage: UIImage?
  @Published private(set) var snapshotHash = ""
  @Published private(set) var snapshotRelayed = false
  @Published private(set) var frameCount: UInt64 = 0
  @Published private(set) var latestSequence: UInt64?
  @Published private(set) var paused = false
  @Published private(set) var snapshotPending = false
  @Published private(set) var awaitingApproval = false
  @Published private(set) var log = ""
  @Published private(set) var phase: CameraMeshPhase = .signedOut

  private enum ConnectionAttempt {
    case discovered(CameraMeshCandidate)
    case card(String)
  }

  private var identity: AukiPeerIdentity?
  private var session: AukiSession?
  private var viewer: CameraViewer?
  private var provisionalPeer: AukiPeer?
  private var provisionalViewer: CameraViewer?
  private var eventTask: Task<Void, Never>?
  private var retryAttempt: ConnectionAttempt?
  private var activeConnectionID: CameraViewerConnectionID?
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
    phase == .ready
  }

  var canConnectDiscovered: Bool {
    phase == .ready
      && discoveredCameras.contains(where: { $0.peerID == selectedCameraPeerID })
  }

  var canConnectCard: Bool {
    phase == .ready
      && !remoteCard.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
  }

  var canRetryConnection: Bool {
    phase == .ready && retryAttempt != nil
  }

  var canPause: Bool {
    phase == .connected && connection != nil && !paused
  }

  var canResume: Bool {
    phase == .connected && connection != nil && paused
  }

  var canRequestSnapshot: Bool {
    phase == .connected && connection != nil && !snapshotPending
  }

  var canDisconnect: Bool {
    phase == .connected && connection != nil
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
      write("Authenticated. Choose the Domain that contains the camera publisher.")
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
        mode: .discoverOnly
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
    return await connect(using: .discovered(candidate))
  }

  @discardableResult
  func connectPastedCard() async -> Bool {
    guard canConnectCard else { return false }
    let card = remoteCard.trimmingCharacters(in: .whitespacesAndNewlines)
    return await connect(using: .card(card))
  }

  @discardableResult
  func retryConnection() async -> Bool {
    guard canRetryConnection, let attempt = retryAttempt else { return false }
    return await connect(using: attempt)
  }

  @discardableResult
  func pause() async -> Bool {
    guard canPause, let viewer, let operationConnectionID = activeConnectionID else {
      return false
    }
    let operationGeneration = generation
    phase = .controlling
    do {
      try await viewer.pause()
      guard generation == operationGeneration, self.viewer === viewer,
        activeConnectionID == operationConnectionID
      else { return false }
      guard connection != nil else {
        phase = .ready
        return false
      }
      paused = true
      phase = .connected
      return true
    } catch {
      guard generation == operationGeneration, self.viewer === viewer,
        activeConnectionID == operationConnectionID
      else { return false }
      phase = connection == nil ? .ready : .connected
      write(error)
      return false
    }
  }

  @discardableResult
  func resume() async -> Bool {
    guard canResume, let viewer, let operationConnectionID = activeConnectionID else {
      return false
    }
    let operationGeneration = generation
    phase = .controlling
    do {
      try await viewer.resume()
      guard generation == operationGeneration, self.viewer === viewer,
        activeConnectionID == operationConnectionID
      else { return false }
      guard connection != nil else {
        phase = .ready
        return false
      }
      paused = false
      phase = .connected
      return true
    } catch {
      guard generation == operationGeneration, self.viewer === viewer,
        activeConnectionID == operationConnectionID
      else { return false }
      phase = connection == nil ? .ready : .connected
      write(error)
      return false
    }
  }

  @discardableResult
  func requestSnapshot() async -> Bool {
    guard canRequestSnapshot, let viewer, let operationConnectionID = activeConnectionID else {
      return false
    }
    let operationGeneration = generation
    snapshotPending = true
    phase = .controlling
    do {
      _ = try await viewer.requestSnapshot()
      guard generation == operationGeneration, self.viewer === viewer,
        activeConnectionID == operationConnectionID
      else { return false }
      phase = connection == nil ? .ready : .connected
      return true
    } catch {
      guard generation == operationGeneration, self.viewer === viewer,
        activeConnectionID == operationConnectionID
      else { return false }
      snapshotPending = false
      phase = connection == nil ? .ready : .connected
      write(error)
      return false
    }
  }

  func disconnect() async {
    guard canDisconnect, let viewer else { return }
    let operationGeneration = generation
    phase = .disconnecting
    await viewer.disconnect(reason: "Disconnected by operator")
    guard generation == operationGeneration, self.viewer === viewer else { return }
    resetRemotePresentation()
    retryAttempt = nil
    awaitingApproval = false
    phase = .ready
    write("Camera disconnected.")
  }

  func stop(reason: String = "Viewer stopped") async {
    guard canStop else { return }
    phase = .stopping
    generation += 1

    let mounted = viewer ?? provisionalViewer
    let peer = mounted == nil ? provisionalPeer : nil
    let observing = eventTask
    viewer = nil
    provisionalViewer = nil
    provisionalPeer = nil
    eventTask = nil
    observing?.cancel()

    do {
      if let mounted {
        try await mounted.close()
      } else if let peer {
        try await peer.shutdown()
      }
      await observing?.value
      write(reason)
    } catch {
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
    retryAttempt = nil
    awaitingApproval = false
    resetRemotePresentation(clearSnapshot: true)
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
      _ = await self.retryConnection()
    }
  }

  private func connect(using attempt: ConnectionAttempt) async -> Bool {
    guard phase == .ready, let viewer else { return false }
    let operationGeneration = generation
    phase = .connecting
    retryAttempt = attempt
    awaitingApproval = false
    resetRemotePresentation(clearSnapshot: true)

    do {
      switch attempt {
      case .discovered(let candidate):
        try await viewer.connect(candidate: candidate)
      case .card(let card):
        try await viewer.connect(cardJSON: card)
      }
      guard generation == operationGeneration, self.viewer === viewer else {
        await viewer.disconnect(reason: "Connection superseded")
        return false
      }
      retryAttempt = nil
      awaitingApproval = false
      phase = .connected
      write("Camera authenticated. Waiting for JPEG frames.")
      return true
    } catch {
      guard generation == operationGeneration, self.viewer === viewer else { return false }
      let message = error.localizedDescription
      awaitingApproval = isCameraApprovalRequired(message)
      phase = .ready
      write(error)
      if awaitingApproval {
        write("The publisher received this viewer Peer ID. Approve it there, then select Retry.")
        print("AUKI_IOS_CAMERA_APPROVAL_REQUIRED VIEWER_PEER=\(localPeerID)")
      }
      return false
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
      guard connectionID == nil || connectionID == activeConnectionID else { return }
      write(message)

    case .connected(let connectionID, let connected):
      guard phase == .connecting || phase == .connected || phase == .ready else { return }
      activeConnectionID = connectionID
      connection = connected
      paused = false
      awaitingApproval = false
      if phase == .connecting || phase == .ready { phase = .connected }
      write("Connected to \(connected.name) (\(connected.runtime)).")
      print(
        "AUKI_IOS_CAMERA_CONNECTED peer=\(connected.peerID) runtime=\(connected.runtime)"
      )

    case .frame(let connectionID, let frame):
      guard connectionID == activeConnectionID else { return }
      guard let image = UIImage(data: frame.jpeg) else {
        write("UIKit could not decode camera JPEG sequence \(frame.sequence).")
        return
      }
      latestFrameImage = image
      frameCount &+= 1
      latestSequence = frame.sequence
      if frameCount == 1 {
        print(
          "AUKI_IOS_CAMERA_FRAME peer=\(connection?.peerID ?? "unknown") "
            + "sequence=\(frame.sequence) bytes=\(frame.jpeg.count)"
        )
      }
      if runAcceptanceFlow, frameCount >= 2, !automationAcceptanceStarted {
        automationAcceptanceStarted = true
        Task { [weak self] in await self?.runAutomatedAcceptanceFlow() }
      } else if snapshotAfterFirstFrame && !automationSnapshotRequested {
        automationSnapshotRequested = true
        Task { [weak self] in _ = await self?.requestSnapshot() }
      }

    case .snapshot(let connectionID, let snapshot):
      guard connectionID == activeConnectionID else { return }
      snapshotPending = false
      snapshotHash = snapshot.sha256
      snapshotRelayed = snapshot.relayed
      if let image = UIImage(data: snapshot.jpeg) {
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
      guard connectionID == activeConnectionID else { return }
      resetRemotePresentation()
      if phase != .stopping { phase = .ready }
      write(reason)
      print("AUKI_IOS_CAMERA_DISCONNECTED")

    case .failed(let connectionID, let reason):
      guard connectionID == nil || connectionID == activeConnectionID else { return }
      write(reason)
      if reason.localizedCaseInsensitiveContains("snapshot") {
        snapshotPending = false
      }
      if reason.localizedCaseInsensitiveContains("camera stream") {
        resetRemotePresentation()
        if phase != .stopping { phase = .ready }
      }
      print("AUKI_IOS_CAMERA_FAILURE \(reason)")
    }
  }

  private func resetRemotePresentation(clearSnapshot: Bool = false) {
    activeConnectionID = nil
    connection = nil
    latestFrameImage = nil
    frameCount = 0
    latestSequence = nil
    paused = false
    snapshotPending = false
    automationSnapshotRequested = false
    automationAcceptanceStarted = false
    if clearSnapshot {
      snapshotImage = nil
      snapshotHash = ""
      snapshotRelayed = false
    }
  }

  private func write(_ value: some StringProtocol) {
    log = String("\(log)\(value)\n".suffix(12_288))
  }

  private func write(_ error: Error) {
    write(error.localizedDescription)
  }

  private func runAutomatedAcceptanceFlow() async {
    guard await pause() else { return }
    try? await Task.sleep(for: .milliseconds(500))
    guard !Task.isCancelled, await resume() else { return }
    automationSnapshotRequested = true
    _ = await requestSnapshot()
  }
}
