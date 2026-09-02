import AukiPortableEcho
import Combine
import Foundation

@MainActor
final class EchoModel: ObservableObject {
    enum Phase: String {
        case signedOut = "Signed out"
        case authenticating = "Authenticating"
        case authenticated = "Choose a Domain"
        case starting = "Starting peer"
        case running = "Peer ready"
        case stopping = "Stopping"
    }

    @Published var email = ""
    @Published var password = ""
    @Published var selectedDomainID = ""
    @Published var advertisePeer = true
    @Published var selectedDiscoveredPeerID = ""
    @Published var remoteCard = ""
    @Published var message = "hello from Swift"
    @Published private(set) var domains: [AukiDomain] = []
    @Published private(set) var discoveredPeers: [AukiDiscoveryCandidate] = []
    @Published private(set) var localCard = ""
    @Published private(set) var log = ""
    @Published private(set) var phase: Phase = .signedOut

    private var identity: AukiPeerIdentity?
    private var session: AukiSession?
    private var peer: AukiPeer?
    private var echo: AukiEcho?
    private var provisionalPeer: AukiPeer?
    private var provisionalEcho: AukiEcho?
    private var receiveTask: Task<Void, Never>?
    private var automationStarted = false
    private var stopAfterReceive = false
    private var generation = 0

    var canLogin: Bool {
        phase == .signedOut && !email.isEmpty && !password.isEmpty
    }

    var canStart: Bool {
        phase == .authenticated && !selectedDomainID.isEmpty
    }

    var canSend: Bool {
        phase == .running && !remoteCard.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !message.isEmpty
    }

    var canRefreshDiscovery: Bool { phase == .running }

    var canSendDiscovered: Bool {
        phase == .running && !selectedDiscoveredPeerID.isEmpty && !message.isEmpty
    }

    @discardableResult
    func login() async -> Bool {
        guard canLogin else { return false }
        phase = .authenticating
        defer { password = "" }
        do {
            let authenticated = try await AukiSession.loginDev(email: email, password: password)
            let choices = try await authenticated.accessibleDomains()
            guard !choices.isEmpty else { throw DemoError("This User has no accessible Domains") }
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
        var startedEcho: AukiEcho?
        do {
            let peerIdentity = identity ?? AukiPeerIdentity.generate()
            identity = peerIdentity
            write("Starting ephemeral Peer ID \(peerIdentity.peerId())…")
            let runningPeer = try await session.startPeerWithDiscovery(
                domainId: selectedDomainID,
                identity: peerIdentity,
                mode: advertisePeer ? .discoverAndAdvertise : .discoverOnly
            )
            startedPeer = runningPeer
            guard generation == operationGeneration, phase == .starting else {
                try? await runningPeer.shutdown()
                return false
            }
            provisionalPeer = runningPeer
            let mounted = try await AukiEcho.mount(peer: runningPeer)
            startedEcho = mounted
            provisionalEcho = mounted
            guard generation == operationGeneration, phase == .starting else {
                try? await mounted.close()
                try? await runningPeer.shutdown()
                return false
            }
            let card = try peerCardToJson(card: mounted.card())

            self.session = nil
            provisionalEcho = nil
            provisionalPeer = nil
            peer = runningPeer
            echo = mounted
            localCard = card
            phase = .running
            beginReceiving(from: mounted)
            write("Peer ready: \(runningPeer.peerId())")
            print("AUKI_IOS_READY PEER_CARD=\(card)")
            return true
        } catch {
            provisionalEcho = nil
            provisionalPeer = nil
            if let startedEcho { try? await startedEcho.close() }
            if let startedPeer { try? await startedPeer.shutdown() }
            guard generation == operationGeneration else { return false }
            phase = .authenticated
            write(error)
            return false
        }
    }

    @discardableResult
    func refreshDiscovery() async -> Bool {
        guard canRefreshDiscovery, let runningPeer = peer, let mounted = echo else { return false }
        do {
            let candidates = try await runningPeer.discoverProtocol(
                protocolId: mounted.protocol()
            )
            guard let currentPeer = peer, currentPeer === runningPeer, phase == .running else {
                return false
            }
            discoveredPeers = candidates
            if !candidates.contains(where: { $0.peerId == selectedDiscoveredPeerID }) {
                selectedDiscoveredPeerID = candidates.first?.peerId ?? ""
            }
            write("Discovered \(candidates.count) Echo peer(s); candidates remain untrusted until exact dial.")
            return true
        } catch {
            write(error)
            return false
        }
    }

    @discardableResult
    func sendDiscovered() async -> Bool {
        guard
            canSendDiscovered,
            let runningPeer = peer,
            let mounted = echo,
            let candidate = discoveredPeers.first(where: {
                $0.peerId == selectedDiscoveredPeerID
            })
        else { return false }

        let routes = nativeDiscoveryRoutes(candidate.routes)
        guard !routes.isEmpty else {
            write("Discovered peer has no native-compatible route")
            return false
        }
        var failures: [String] = []
        for route in routes {
            do {
                let receipt = try await mounted.sendExact(
                    target: AukiPeerTarget(
                        domainId: runningPeer.domainId(),
                        peerId: candidate.peerId,
                        route: route
                    ),
                    payload: Data(message.utf8)
                )
                let payload = String(decoding: receipt.payload, as: UTF8.self)
                write("Sent to discovered peer \(receipt.remotePeerId): \(payload)")
                print(
                    "AUKI_IOS_ECHO_SENT peer=\(receipt.remotePeerId) "
                        + "relayed=\(receipt.relayed) payload=\(payload)"
                )
                return true
            } catch {
                failures.append("\(route): \(error.localizedDescription)")
            }
        }
        write("Every discovered route failed: \(failures.joined(separator: "; "))")
        return false
    }

    @discardableResult
    func send() async -> Bool {
        guard canSend, let mounted = echo else { return false }
        do {
            let card = try peerCardFromJson(
                json: remoteCard.trimmingCharacters(in: .whitespacesAndNewlines)
            )
            let receipt = try await mounted.sendExact(
                target: nativePeerTarget(card: card, requiredProtocol: "/example/echo/1.0.0"),
                payload: Data(message.utf8)
            )
            guard receipt.relayed else { throw DemoError("Echo did not use the relay") }
            let payload = String(decoding: receipt.payload, as: UTF8.self)
            write("Sent to \(receipt.remotePeerId): \(payload)")
            print("AUKI_IOS_ECHO_SENT peer=\(receipt.remotePeerId) relayed=true payload=\(payload)")
            return true
        } catch {
            write(error)
            return false
        }
    }

    func stop(reason: String = "Peer stopped") async {
        guard phase == .starting || phase == .running else { return }
        phase = .stopping
        generation += 1
        let runningPeer = peer ?? provisionalPeer
        let mounted = echo ?? provisionalEcho
        let pendingReceive = receiveTask
        peer = nil
        echo = nil
        provisionalPeer = nil
        provisionalEcho = nil
        receiveTask = nil

        var failures: [String] = []
        if let mounted {
            do { try await mounted.close() } catch { failures.append(error.localizedDescription) }
        }
        if let runningPeer {
            do { try await runningPeer.shutdown() } catch { failures.append(error.localizedDescription) }
        }
        await pendingReceive?.value

        session = nil
        domains = []
        discoveredPeers = []
        selectedDiscoveredPeerID = ""
        selectedDomainID = ""
        localCard = ""
        phase = .signedOut
        write(failures.isEmpty ? reason : failures.joined(separator: "\n"))
        print("AUKI_IOS_STOPPED")
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
        message = environment["AUKI_IOS_MESSAGE"] ?? message
        stopAfterReceive = environment["AUKI_IOS_STOP_AFTER_RECEIVE"] == "1"
        Task {
            guard await login() else { return }
            selectedDomainID = automationDomain
            guard domains.contains(where: { $0.id == automationDomain }) else {
                write("Configured automation Domain is not accessible")
                return
            }
            guard await start() else { return }
            if !remoteCard.isEmpty { _ = await send() }
        }
    }

    private func beginReceiving(from mounted: AukiEcho) {
        receiveTask = Task { [weak self] in
            while let self, self.echo === mounted {
                do {
                    let receipt = try await mounted.nextServed()
                    guard self.echo === mounted else { return }
                    let payload = String(decoding: receipt.payload, as: UTF8.self)
                    self.write("Received from \(receipt.remotePeerId): \(payload)")
                    print("AUKI_IOS_ECHO_SERVED peer=\(receipt.remotePeerId) payload=\(payload)")
                    if self.stopAfterReceive {
                        Task { await self.stop(reason: "Automation completed") }
                        return
                    }
                } catch {
                    if self.echo === mounted { self.write(error) }
                    return
                }
            }
        }
    }

    private func nativeDiscoveryRoutes(_ routes: [String]) -> [String] {
        routes
            .filter { !$0.split(separator: "/").contains("wss") }
            .sorted { left, right in
                let leftRelay = left.contains("/p2p-circuit/")
                let rightRelay = right.contains("/p2p-circuit/")
                return leftRelay == rightRelay ? left < right : leftRelay
            }
    }

    private func write(_ value: some StringProtocol) {
        log = String("\(log)\(value)\n".suffix(8_192))
    }

    private func write(_ error: Error) {
        write(error.localizedDescription)
    }
}

private struct DemoError: LocalizedError {
    let message: String
    init(_ message: String) { self.message = message }
    var errorDescription: String? { message }
}
