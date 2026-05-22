import XCTest
import AukiProto
import auki_network

final class GeneratedBindingsSmokeTests: XCTestCase {
    func testGeneratedBindingsConstructMessageNode() throws {
        let seed = Data(repeating: 3, count: 32)
        let node = try AukiMessageNode.fromWalletSeed(
            seed: seed,
            listenAddrs: [],
            agentVersion: "auki-ios-test-app-tests/0.1"
        )
        XCTAssertFalse(node.peerId().isEmpty)
        node.shutdown()
    }

    func testGeneratedProtoSerializesEnvelope() throws {
        var envelope = Auki_Message_MessageEnvelope()
        envelope.typeURL = "auki.test/ping"
        envelope.body = Data([1, 2, 3])
        envelope.requestID = "req-1"
        let bytes = try envelope.serializedData()
        let decoded = try Auki_Message_MessageEnvelope(serializedBytes: bytes)
        XCTAssertEqual(decoded.requestID, "req-1")
    }
}
