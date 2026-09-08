import AukiSDK
import AukiStandardPlayground
import Combine
import Foundation

enum StandardDiscoveryChoice: String, CaseIterable, Identifiable {
  case discoverAndAdvertise = "Discover and advertise"
  case discoverOnly = "Discover only (stay private)"

  var id: String { rawValue }

  var sdkMode: AukiDiscoveryMode {
    switch self {
    case .discoverAndAdvertise: .discoverAndAdvertise
    case .discoverOnly: .discoverOnly
    }
  }
}

@MainActor
final class StandardProtocolsModel: ObservableObject {
  enum Phase: String {
    case signedOut = "Signed out"
    case authenticating = "Authenticating"
    case authenticated = "Choose a Domain"
    case starting = "Starting peer"
    case running = "Peer ready"
    case discovering = "Discovering peers"
    case probing = "Probing protocols"
    case stopping = "Stopping"
  }

  @Published var email = ""
  @Published var password = ""
  @Published var selectedDomainID = ""
  @Published var discoveryChoice: StandardDiscoveryChoice = .discoverAndAdvertise
  @Published var selectedDiscoveryProtocol = ""
  @Published var selectedDiscoveredPeerID = ""
  @Published var remoteCard = ""
  @Published private(set) var domains: [AukiDomain] = []
  @Published private(set) var discoveryProtocols: [String] = []
  @Published private(set) var discoveredPeers: [StandardDiscoveredPeer] = []
  @Published private(set) var localCard = ""
  @Published private(set) var log = ""
  @Published private(set) var phase: Phase = .signedOut

  private var identity: AukiPeerIdentity?
  private var session: AukiSession?
  private var playground: StandardPlayground?
  private var provisionalPeer: AukiPeer?
  private var provisionalPlayground: StandardPlayground?
  private var automationStarted = false
  private var generation = 0

  var canLogin: Bool {
    phase == .signedOut && !email.isEmpty && !password.isEmpty
  }

  var canStart: Bool {
    phase == .authenticated && !selectedDomainID.isEmpty
  }

  var canProbe: Bool {
    phase == .running && !remoteCard.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
  }

  var canDiscover: Bool {
    phase == .running
  }

  var canProbeDiscovered: Bool {
    phase == .running
      && discoveredPeers.contains(where: {
        $0.peerID == selectedDiscoveredPeerID && isProbeable($0)
      })
  }

  func isProbeable(_ candidate: StandardDiscoveredPeer) -> Bool {
    discoveryProtocols.allSatisfy(candidate.servedProtocols.contains)
      && candidate.routes.contains(where: {
        $0.contains("/tcp/") && !$0.contains("/wss/")
      })
  }

  @discardableResult
  func login() async -> Bool {
    guard canLogin else { return false }
    phase = .authenticating
    defer { password = "" }
    do {
      let authenticated = try await AukiSession.loginDev(email: email, password: password)
      let choices = try await authenticated.accessibleDomains()
      guard !choices.isEmpty else {
        throw StandardPlaygroundError("This User has no accessible Domains")
      }
      session = authenticated
      domains = choices
      if !choices.contains(where: { $0.id == selectedDomainID }) {
        selectedDomainID = choices[0].id
      }
      phase = .authenticated
      write("Authenticated. Choose a Domain.")
      return true
    } catch {
      session = nil
      domains = []
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
    var startedPlayground: StandardPlayground?
    do {
      let peerIdentity = identity ?? AukiPeerIdentity.generate()
      identity = peerIdentity
      write("Starting ephemeral Peer ID \(peerIdentity.peerId())…")
      let peer = try await session.startPeerWithDiscovery(
        domainId: selectedDomainID,
        identity: peerIdentity,
        mode: discoveryChoice.sdkMode
      )
      startedPeer = peer
      guard generation == operationGeneration, phase == .starting else {
        try? await peer.shutdown()
        return false
      }
      provisionalPeer = peer
      let mounted = try await StandardPlayground.mount(
        peer: peer,
        nodeName: ProcessInfo.processInfo.environment["AUKI_IOS_NODE_NAME"]
          ?? "swift-playground"
      )
      startedPlayground = mounted
      provisionalPeer = nil
      guard generation == operationGeneration, phase == .starting else {
        try? await mounted.close()
        return false
      }
      provisionalPlayground = mounted
      let card = try await mounted.cardJSON()
      guard generation == operationGeneration, phase == .starting else {
        try? await mounted.close()
        return false
      }

      self.session = nil
      provisionalPlayground = nil
      playground = mounted
      localCard = card
      discoveryProtocols = await mounted.protocols()
      selectedDiscoveryProtocol = ""
      discoveredPeers = []
      selectedDiscoveredPeerID = ""
      phase = .running
      write("Seven protocol IDs across all six families are ready.")
      print("AUKI_IOS_STANDARD_READY PEER_CARD=\(card)")
      return true
    } catch {
      provisionalPeer = nil
      provisionalPlayground = nil
      if let startedPlayground {
        // The mounted bundle owns the peer and closes its adapters before shutdown.
        try? await startedPlayground.close()
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
  func discoverPeers() async -> Bool {
    guard canDiscover, let mounted = playground else { return false }
    let operationGeneration = generation
    phase = .discovering
    do {
      let candidates = try await mounted.discover(
        protocolID: selectedDiscoveryProtocol.isEmpty ? nil : selectedDiscoveryProtocol
      )
      guard generation == operationGeneration, playground === mounted else { return false }
      discoveredPeers = candidates
      if !candidates.contains(where: {
        $0.peerID == selectedDiscoveredPeerID && isProbeable($0)
      }) {
        selectedDiscoveredPeerID = candidates.first(where: isProbeable)?.peerID ?? ""
      }
      write("Discovery returned \(candidates.count) current candidate(s).")
      write("Candidates remain untrusted until an exact protocol connection authenticates them.")
      phase = .running
      return true
    } catch {
      guard generation == operationGeneration, playground === mounted else { return false }
      phase = .running
      write(error)
      return false
    }
  }

  @discardableResult
  func probeDiscovered() async -> Bool {
    guard
      canProbeDiscovered,
      let mounted = playground,
      let candidate = discoveredPeers.first(where: { $0.peerID == selectedDiscoveredPeerID })
    else { return false }
    let operationGeneration = generation
    phase = .probing
    let report = await mounted.probeAll(candidate: candidate)
    guard generation == operationGeneration, playground === mounted else { return false }
    write(report: report)
    phase = .running
    print(
      "AUKI_IOS_STANDARD_PROBE ok=\(report.ok) "
        + "peer=\(report.targetPeerID) checks=\(checkSummary(report))"
    )
    return report.ok
  }

  @discardableResult
  func probeAll() async -> Bool {
    guard canProbe, let mounted = playground else { return false }
    let operationGeneration = generation
    phase = .probing
    let report = await mounted.probeAll(cardJSON: remoteCard)
    guard generation == operationGeneration, playground === mounted else { return false }
    write(report: report)
    phase = .running
    print(
      "AUKI_IOS_STANDARD_PROBE ok=\(report.ok) "
        + "peer=\(report.targetPeerID) checks=\(checkSummary(report))"
    )
    return report.ok
  }

  func stop(reason: String = "Peer stopped") async {
    guard phase == .starting || phase == .running || phase == .discovering
      || phase == .probing else { return }
    phase = .stopping
    generation += 1
    let mounted = playground ?? provisionalPlayground
    let peer = mounted == nil ? provisionalPeer : nil
    playground = nil
    provisionalPlayground = nil
    provisionalPeer = nil
    do {
      if let mounted {
        try await mounted.close()
      } else if let peer {
        try await peer.shutdown()
      }
      write(reason)
    } catch {
      write(error)
    }

    session = nil
    domains = []
    discoveryProtocols = []
    discoveredPeers = []
    selectedDiscoveryProtocol = ""
    selectedDiscoveredPeerID = ""
    selectedDomainID = ""
    localCard = ""
    phase = .signedOut
    print("AUKI_IOS_STANDARD_STOPPED")
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
    Task {
      guard await login() else { return }
      selectedDomainID = automationDomain
      guard domains.contains(where: { $0.id == automationDomain }) else {
        write("Configured automation Domain is not accessible")
        return
      }
      guard await start() else { return }
      if !remoteCard.isEmpty {
        _ = await probeAll()
        if environment["AUKI_IOS_STOP_AFTER_PROBE"] == "1" {
          await stop(reason: "Automation completed")
        }
      }
    }
  }

  private func checkSummary(_ report: StandardProbeReport) -> String {
    StandardProtocolFamily.allCases
      .map { "\($0.rawValue):\(report.checks[$0] == true)" }
      .joined(separator: ",")
  }

  private func write(report: StandardProbeReport) {
    for family in StandardProtocolFamily.allCases {
      if report.checks[family] == true {
        write("✓ \(family.rawValue)")
      } else {
        write("✗ \(family.rawValue): \(report.errors[family] ?? "unknown failure")")
      }
    }
  }

  private func write(_ value: some StringProtocol) {
    log = String("\(log)\(value)\n".suffix(8_192))
  }

  private func write(_ error: Error) {
    write(error.localizedDescription)
  }
}
