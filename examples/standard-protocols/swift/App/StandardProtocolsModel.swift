import AukiSDK
import AukiStandardPlayground
import Combine
import Foundation

@MainActor
final class StandardProtocolsModel: ObservableObject {
  enum Phase: String {
    case signedOut = "Signed out"
    case authenticating = "Authenticating"
    case authenticated = "Choose a Domain"
    case starting = "Starting peer"
    case running = "Peer ready"
    case probing = "Probing protocols"
    case stopping = "Stopping"
  }

  @Published var email = ""
  @Published var password = ""
  @Published var selectedDomainID = ""
  @Published var remoteCard = ""
  @Published private(set) var domains: [AukiDomain] = []
  @Published private(set) var localCard = ""
  @Published private(set) var log = ""
  @Published private(set) var phase: Phase = .signedOut

  private var identity: AukiPeerIdentity?
  private var session: AukiSession?
  private var playground: StandardPlayground?
  private var automationStarted = false

  var canLogin: Bool {
    phase == .signedOut && !email.isEmpty && !password.isEmpty
  }

  var canStart: Bool {
    phase == .authenticated && !selectedDomainID.isEmpty
  }

  var canProbe: Bool {
    phase == .running && !remoteCard.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
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
    phase = .starting
    var startedPeer: AukiPeer?
    do {
      let peerIdentity = identity ?? AukiPeerIdentity.generate()
      identity = peerIdentity
      write("Starting ephemeral Peer ID \(peerIdentity.peerId())…")
      let peer = try await session.startPeer(
        domainId: selectedDomainID,
        identity: peerIdentity
      )
      startedPeer = peer
      let mounted = try await StandardPlayground.mount(
        peer: peer,
        nodeName: ProcessInfo.processInfo.environment["AUKI_IOS_NODE_NAME"]
          ?? "swift-playground"
      )
      let card = try await mounted.cardJSON()

      self.session = nil
      playground = mounted
      localCard = card
      phase = .running
      write("Seven protocol IDs across all six families are ready.")
      print("AUKI_IOS_STANDARD_READY PEER_CARD=\(card)")
      return true
    } catch {
      if let startedPeer { try? await startedPeer.shutdown() }
      phase = .authenticated
      write(error)
      return false
    }
  }

  @discardableResult
  func probeAll() async -> Bool {
    guard canProbe, let playground else { return false }
    phase = .probing
    let report = await playground.probeAll(cardJSON: remoteCard)
    for family in StandardProtocolFamily.allCases {
      if report.checks[family] == true {
        write("✓ \(family.rawValue)")
      } else {
        write("✗ \(family.rawValue): \(report.errors[family] ?? "unknown failure")")
      }
    }
    phase = .running
    print(
      "AUKI_IOS_STANDARD_PROBE ok=\(report.ok) "
        + "peer=\(report.targetPeerID) checks=\(checkSummary(report))"
    )
    return report.ok
  }

  func stop(reason: String = "Peer stopped") async {
    guard phase == .running || phase == .probing, let mounted = playground else { return }
    phase = .stopping
    playground = nil
    do {
      try await mounted.close()
      write(reason)
    } catch {
      write(error)
    }

    session = nil
    domains = []
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

  private func write(_ value: some StringProtocol) {
    log = String("\(log)\(value)\n".suffix(8_192))
  }

  private func write(_ error: Error) {
    write(error.localizedDescription)
  }
}
