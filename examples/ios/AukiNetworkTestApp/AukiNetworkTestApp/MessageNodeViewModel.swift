import Combine
import Foundation
import AukiProto
import auki_network

@MainActor
final class MessageNodeViewModel: ObservableObject {
    @Published var walletSeedHex = String(repeating: "03", count: 32)
    @Published var peerId = ""
    @Published var listenAddrs = ""
    @Published var browserPeerId = ""
    @Published var browserAddrs = ""
    @Published var eventLog = ""

    private var node: AukiMessageNode?

    func start() {
        do {
            let seed = try Self.hexToData(walletSeedHex)
            let node = try AukiMessageNode.fromWalletSeed(
                seed: seed,
                listenAddrs: ["/ip4/0.0.0.0/udp/0/webrtc-direct"],
                agentVersion: "auki-ios-test-app/0.1"
            )
            self.node = node
            peerId = node.peerId()
            listenAddrs = node.listenAddrs().joined(separator: "\n")
            append("started \(peerId)")
        } catch {
            append("start failed: \(error)")
        }
    }

    func refreshListenAddrs() {
        guard let node else {
            append("node is not started")
            return
        }
        listenAddrs = node.listenAddrs().joined(separator: "\n")
    }

    func dialBrowser() {
        guard let node else {
            append("node is not started")
            return
        }
        let addrs = browserAddrs
            .split(whereSeparator: \.isNewline)
            .map(String.init)
        do {
            try node.dial(peerId: browserPeerId, addrs: addrs)
            append("dial requested \(browserPeerId)")
        } catch {
            append("dial failed: \(error)")
        }
    }

    func sendPing() {
        guard let node else {
            append("node is not started")
            return
        }
        do {
            var envelope = Auki_Message_MessageEnvelope()
            envelope.typeURL = "auki.test/ping"
            envelope.body = Data("hello from ios".utf8)
            envelope.requestID = UUID().uuidString
            let ackBytes = try node.sendMessageEnvelopeBytes(
                peerId: browserPeerId,
                envelope: try envelope.serializedData()
            )
            let ack = try Auki_Message_MessageAck(serializedBytes: ackBytes)
            append("ack \(ack.requestID) accepted=\(ack.accepted) \(ack.detail)")
        } catch {
            append("send failed: \(error)")
        }
    }

    func pollEvent() {
        guard let node else {
            append("node is not started")
            return
        }
        do {
            guard let event = try node.nextEvent() else {
                append("no event")
                return
            }
            let envelope = try Auki_Message_MessageEnvelope(serializedBytes: event.envelope)
            append("message from \(event.peerId): \(envelope.typeURL) \(envelope.requestID)")
        } catch {
            append("poll failed: \(error)")
        }
    }

    func stop() {
        node?.shutdown()
        node = nil
        append("stopped")
    }

    private func append(_ line: String) {
        eventLog = eventLog.isEmpty ? line : "\(eventLog)\n\(line)"
    }

    private static func hexToData(_ hex: String) throws -> Data {
        let cleaned = hex.trimmingCharacters(in: .whitespacesAndNewlines)
        guard cleaned.count % 2 == 0 else {
            throw HexError.invalidLength
        }
        var data = Data()
        var index = cleaned.startIndex
        while index < cleaned.endIndex {
            let next = cleaned.index(index, offsetBy: 2)
            guard let byte = UInt8(cleaned[index..<next], radix: 16) else {
                throw HexError.invalidByte
            }
            data.append(byte)
            index = next
        }
        return data
    }

    enum HexError: Error {
        case invalidLength
        case invalidByte
    }
}
