import AukiSDK
import AukiStandardPlayground
import XCTest

final class AukiStandardPlaygroundTests: XCTestCase {
  func testPeerCardRoundTripAndNativeTarget() throws {
    let target = AukiPeerIdentity.generate().peerId()
    let relay = AukiPeerIdentity.generate().peerId()
    let card = AukiPeerCard(
      version: 1,
      domainId: "DE66FDF4-A830-4017-95DD-5741C30A6D0F",
      peerId: target,
      protocols: StandardFixtures.protocolIDs,
      routes: AukiPeerRoutes(
        tcp: "/dns4/relay.dev.aukiverse.com/tcp/443/p2p/\(relay)/p2p-circuit/p2p/\(target)",
        wss: "/dns4/relay.dev.aukiverse.com/tcp/4443/wss/p2p/\(relay)/p2p-circuit/p2p/\(target)"
      )
    )

    let json = try peerCardToJson(card: card)
    let decoded = try peerCardFromJson(json: json)
    let selected = try nativePeerTarget(
      card: decoded,
      requiredProtocol: "/auki/auth/1/info/1.0.0"
    )

    XCTAssertTrue(json.contains(#""runtime":"swift""#))
    XCTAssertEqual(decoded.domainId, "de66fdf4-a830-4017-95dd-5741c30a6d0f")
    XCTAssertEqual(decoded.protocols, StandardFixtures.protocolIDs)
    XCTAssertEqual(selected.peerId, target)
    XCTAssertEqual(selected.route, decoded.routes.tcp)
  }

  func testFiniteFixtureJSONAndMessageRecords() throws {
    let peerID = AukiPeerIdentity.generate().peerId()
    let info = StandardFixtures.info(peerID: peerID, nodeName: "unit-test")
    let decodedInfo = try StandardFixtures.decode(
      ParticipantInfo.self,
      from: StandardFixtures.json(info)
    )
    XCTAssertEqual(decodedInfo, info)

    let catalog = StandardFixtures.catalogResources(peerID: peerID)
    let decodedCatalog = try StandardFixtures.decode(
      CatalogResourcesSnapshot.self,
      from: StandardFixtures.json(catalog)
    )
    XCTAssertEqual(decodedCatalog, catalog)
    XCTAssertEqual(decodedCatalog.resources.single?.ownerPeerID, peerID)

    let frame = StandardFixtures.frameRegistry(peerID: peerID)
    let decodedFrame = try StandardFixtures.decode(
      FrameRegistryFixture.self,
      from: StandardFixtures.json(frame)
    )
    XCTAssertEqual(decodedFrame, frame)
    XCTAssertEqual(decodedFrame.frameID, StandardFixtures.registryID)

    let channel = StandardFixtures.messageChannel(peerID: peerID)
    XCTAssertEqual(channel.ownerPeerId, peerID)
    XCTAssertEqual(channel.resourceId, StandardFixtures.messageResourceID)
    XCTAssertEqual(channel.clock.peerId, peerID)
    XCTAssertEqual(channel.clock.id, StandardFixtures.messageClockID)
    XCTAssertEqual(channel.clock.hash, StandardFixtures.messageClockHash)
  }

  func testScalarProtobufBytesAreLocked() throws {
    let bytes = StandardFixtures.scalarBytes()
    XCTAssertEqual(bytes, Data([0x09, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x29, 0x40]))
    XCTAssertEqual(try StandardFixtures.scalarValue(from: bytes), 12.5)
    XCTAssertThrowsError(try StandardFixtures.scalarValue(from: Data([0x08, 0x01])))
  }

  func testRegistryListHashInvariantAndEmptyMaps() throws {
    XCTAssertTrue(StandardFixtures.isRegistryHash(String(repeating: "0", count: 32)))
    XCTAssertTrue(StandardFixtures.isRegistryHash("0123456789abcdef0123456789abcdef"))
    XCTAssertFalse(StandardFixtures.isRegistryHash("ABCDEF" + String(repeating: "0", count: 26)))
    XCTAssertFalse(StandardFixtures.isRegistryHash(String(repeating: "0", count: 64)))
    XCTAssertTrue(try StandardFixtures.resourcesAreEmpty(json: #"{"resources":[]}"#))
    XCTAssertFalse(try StandardFixtures.resourcesAreEmpty(json: #"{"resources":[{}]}"#))
  }
}

extension Collection {
  fileprivate var single: Element? {
    count == 1 ? first : nil
  }
}
