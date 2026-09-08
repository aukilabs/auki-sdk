import AukiPortableEcho
import XCTest

final class AukiPortableEchoTests: XCTestCase {
    func testIdentityRoundTripAndMalformedIdentity() throws {
        let identity = AukiPeerIdentity.generate()
        let restored = try AukiPeerIdentity.fromEncoded(encoded: identity.encoded())

        XCTAssertEqual(restored.peerId(), identity.peerId())
        XCTAssertThrowsError(try AukiPeerIdentity.fromEncoded(encoded: Data([0, 1, 2])))
    }

    func testPeerCardRoundTripAndNativeTarget() throws {
        let target = AukiPeerIdentity.generate().peerId()
        let relay = AukiPeerIdentity.generate().peerId()
        let card = AukiPeerCard(
            version: 1,
            domainId: "DE66FDF4-A830-4017-95DD-5741C30A6D0F",
            peerId: target,
            protocols: [AukiPortableEchoModule.protocolId],
            routes: AukiPeerRoutes(
                tcp: "/dns4/relay.dev.aukiverse.com/tcp/443/p2p/\(relay)/p2p-circuit/p2p/\(target)",
                wss: "/dns4/relay.dev.aukiverse.com/tcp/4443/wss/p2p/\(relay)/p2p-circuit/p2p/\(target)"
            )
        )

        let json = try peerCardToJson(card: card)
        let decoded = try peerCardFromJson(
            json: json.replacingOccurrences(
                of: "\"runtime\":\"swift\"",
                with: "\"runtime\":\"ios\""
            )
        )
        let selected = try nativePeerTarget(
            card: decoded,
            requiredProtocol: "/example/echo/1.0.0"
        )

        XCTAssertEqual(decoded.domainId, "de66fdf4-a830-4017-95dd-5741c30a6d0f")
        XCTAssertEqual(selected.peerId, target)
        XCTAssertEqual(selected.route, decoded.routes.tcp)
    }

    func testPeerCardRejectsArbitraryRoutes() {
        let peer = AukiPeerIdentity.generate().peerId()
        let card = AukiPeerCard(
            version: 1,
            domainId: "de66fdf4-a830-4017-95dd-5741c30a6d0f",
            peerId: peer,
            protocols: [AukiPortableEchoModule.protocolId],
            routes: AukiPeerRoutes(
                tcp: "/ip4/127.0.0.1/tcp/4001/p2p/\(peer)",
                wss: "/ip4/127.0.0.1/tcp/4002/p2p/\(peer)"
            )
        )

        XCTAssertThrowsError(
            try nativePeerTarget(card: card, requiredProtocol: "/example/echo/1.0.0")
        )
    }

    func testPortableUmbrellaExposesDiscoveryCandidates() {
        let peer = AukiPeerIdentity.generate().peerId()
        let relay = AukiPeerIdentity.generate().peerId()
        let candidate = AukiDiscoveryCandidate(
            peerId: peer,
            routes: [
                "/dns4/relay.dev.aukiverse.com/tcp/443/p2p/\(relay)/p2p-circuit/p2p/\(peer)",
                "/dns4/relay.dev.aukiverse.com/tcp/4443/wss/p2p/\(relay)/p2p-circuit/p2p/\(peer)",
            ],
            servedProtocols: [AukiPortableEchoModule.protocolId],
            expiresAt: "2026-09-02T00:00:00Z",
            source: .ddsTracker
        )

        XCTAssertEqual(candidate.peerId, peer)
        XCTAssertEqual(candidate.servedProtocols, ["/example/echo/1.0.0"])
        XCTAssertEqual(candidate.routes.count, 2)
    }
}
