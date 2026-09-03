import AukiCameraMesh
import XCTest

final class CameraContractTests: XCTestCase {
  private let domainID = "de66fdf4-a830-4017-95dd-5741c30a6d0f"
  private let sensorHash = "11111111111111111111111111111111"
  private let clockHash = "22222222222222222222222222222222"
  private let frameHash = "33333333333333333333333333333333"

  func testDiscoveryChoosesCircuitTCPAndRejectsIncompletePublisher() throws {
    let peerID = AukiPeerIdentity.generate().peerId()
    let relayID = AukiPeerIdentity.generate().peerId()
    let direct = "/dns4/direct.example.com/tcp/4001/p2p/\(peerID)"
    let circuit = "/dns4/relay.dev.aukiverse.com/tcp/443/p2p/\(relayID)/p2p-circuit/p2p/\(peerID)"
    let wss = "/dns4/relay.dev.aukiverse.com/tcp/4443/wss/p2p/\(relayID)/p2p-circuit/p2p/\(peerID)"
    let candidate = CameraMeshCandidate(
      peerID: peerID,
      routes: [direct, circuit, wss],
      servedProtocols: CameraMeshContract.publisherProtocolIDs,
      expiresAt: "2026-09-02T00:00:00Z",
      source: "dds_tracker"
    )

    let target = try nativeCameraTarget(candidate: candidate, domainID: domainID)
    XCTAssertEqual(target.peerId, peerID)
    XCTAssertEqual(target.route, circuit)

    let incomplete = CameraMeshCandidate(
      peerID: peerID,
      routes: [circuit],
      servedProtocols: [CameraMeshContract.streamProtocolID],
      expiresAt: candidate.expiresAt,
      source: candidate.source
    )
    XCTAssertThrowsError(try nativeCameraTarget(candidate: incomplete, domainID: domainID))
  }

  func testDiscoveryFallsBackWhenCircuitPairFailsRustValidation() throws {
    let peerID = AukiPeerIdentity.generate().peerId()
    let direct = "/dns4/direct.example.com/tcp/4001/p2p/\(peerID)"
    let invalidRelay = "not-a-peer-id"
    let invalidCircuit =
      "/dns4/relay.dev.aukiverse.com/tcp/443/p2p/\(invalidRelay)/p2p-circuit/p2p/\(peerID)"
    let invalidWSS =
      "/dns4/relay.dev.aukiverse.com/tcp/4443/wss/p2p/\(invalidRelay)/p2p-circuit/p2p/\(peerID)"
    let candidate = CameraMeshCandidate(
      peerID: peerID,
      routes: [invalidCircuit, invalidWSS, direct],
      servedProtocols: CameraMeshContract.publisherProtocolIDs,
      expiresAt: "2026-09-02T00:00:00Z",
      source: "dds_tracker"
    )

    XCTAssertEqual(
      try nativeCameraTarget(candidate: candidate, domainID: domainID).route,
      direct
    )
  }

  func testPastedCardUsesValidatedTCPRouteAndSameDomain() throws {
    let peerID = AukiPeerIdentity.generate().peerId()
    let relayID = AukiPeerIdentity.generate().peerId()
    let tcp = "/dns4/relay.dev.aukiverse.com/tcp/443/p2p/\(relayID)/p2p-circuit/p2p/\(peerID)"
    let wss = "/dns4/relay.dev.aukiverse.com/tcp/4443/wss/p2p/\(relayID)/p2p-circuit/p2p/\(peerID)"
    let cardJSON = try peerCardToJson(
      card: AukiPeerCard(
        version: 1,
        domainId: domainID,
        peerId: peerID,
        protocols: CameraMeshContract.publisherProtocolIDs,
        routes: AukiPeerRoutes(tcp: tcp, wss: wss)
      ))

    XCTAssertEqual(
      try nativeCameraTarget(cardJSON: cardJSON, domainID: domainID).route,
      tcp
    )
    XCTAssertThrowsError(
      try nativeCameraTarget(
        cardJSON: cardJSON,
        domainID: "00000000-0000-0000-0000-000000000002"
      ))
  }

  func testCameraMetadataAndStreamManifestMatchLockedContract() throws {
    let peerID = AukiPeerIdentity.generate().peerId()
    let catalog = try parseCameraCatalog(
      infoJSON: infoJSON(peerID: peerID),
      catalogJSON: catalogJSON(peerID: peerID),
      expectedPeerID: peerID
    )
    let metadata = try resolveCameraMetadata(
      catalog: catalog,
      sensorJSON: sensorJSON(peerID: peerID),
      clockJSON: clockJSON(peerID: peerID),
      frameJSON: frameJSON(peerID: peerID)
    )

    XCTAssertEqual(metadata.info.name, "unit camera")
    XCTAssertEqual(metadata.controlChannel.ownerPeerId, peerID)
    XCTAssertNoThrow(
      try validateStreamManifest(streamManifest(peerID: peerID), metadata: metadata)
    )

    var wrong = streamManifest(peerID: peerID)
    wrong.expectedRateHz = 30
    XCTAssertThrowsError(try validateStreamManifest(wrong, metadata: metadata))
  }

  func testEmptyCatalogIsTheExplicitApprovalBoundary() throws {
    let peerID = AukiPeerIdentity.generate().peerId()
    XCTAssertThrowsError(
      try parseCameraCatalog(
        infoJSON: infoJSON(peerID: peerID),
        catalogJSON: #"{"resources":[]}"#,
        expectedPeerID: peerID
      )
    ) { error in
      XCTAssertTrue(error.localizedDescription.contains("approval_required"))
    }
  }

  func testInfoUsesUInt64AndStrictWireKeys() throws {
    let peerID = AukiPeerIdentity.generate().peerId()
    let maximumJSON = infoJSON(peerID: peerID).replacingOccurrences(
      of: #""session_now_ns":7"#,
      with: #""session_now_ns":18446744073709551615"#
    )
    let parsed = try parseCameraCatalog(
      infoJSON: maximumJSON,
      catalogJSON: catalogJSON(peerID: peerID),
      expectedPeerID: peerID
    )
    XCTAssertEqual(parsed.info.sessionNowNs, UInt64.max)

    let unknownKeyJSON = String(infoJSON(peerID: peerID).dropLast()) + ",\"extra\":true}"
    XCTAssertThrowsError(
      try parseCameraCatalog(
        infoJSON: unknownKeyJSON,
        catalogJSON: catalogJSON(peerID: peerID),
        expectedPeerID: peerID
      ))
    XCTAssertThrowsError(
      try parseCameraCatalog(
        infoJSON: infoJSON(peerID: peerID),
        catalogJSON: #"{"resources":[],"extra":true}"#,
        expectedPeerID: peerID
      ))

    let unknownChannelKey = catalogJSON(peerID: peerID).replacingOccurrences(
      of: #""resource_id":"camera/control","clock":"#,
      with: #""resource_id":"camera/control","extra":true,"clock":"#
    )
    XCTAssertThrowsError(
      try parseCameraCatalog(
        infoJSON: infoJSON(peerID: peerID),
        catalogJSON: unknownChannelKey,
        expectedPeerID: peerID
      ))
  }

  func testSnapshotWireCarriesBothReverseRoutesAndValidatesReady() throws {
    let peerID = AukiPeerIdentity.generate().peerId()
    // This direct TCP + WSS shape is intentionally the same non-circuit shape
    // accepted by the Rust Camera Mesh contract fixture.
    let tcp = "/ip4/127.0.0.1/tcp/9000/p2p/\(peerID)"
    let wss = "/dns4/relay.example.com/tcp/443/wss/p2p/\(peerID)"
    let request = try makeSnapshotRequest(
      requestID: "snapshot-1",
      replyPeerID: peerID,
      replyRoutes: [tcp, wss],
      clock: AukiMessageClockReference(
        peerId: peerID,
        id: CameraMeshContract.clockID,
        hash: clockHash
      )
    )
    let object = try XCTUnwrap(
      JSONSerialization.jsonObject(with: request) as? [String: Any]
    )
    let reply = try XCTUnwrap(object["reply"] as? [String: Any])
    let target = try XCTUnwrap(reply["target"] as? [String: Any])
    let channel = try XCTUnwrap(reply["channel"] as? [String: Any])
    let encodedClock = try XCTUnwrap(channel["clock"] as? [String: Any])
    XCTAssertEqual(target["routes"] as? [String], [tcp, wss])
    XCTAssertEqual(target["peerId"] as? String, peerID)
    XCTAssertEqual(channel["variant"] as? String, "message_channel")
    XCTAssertEqual(channel["owner_peer_id"] as? String, peerID)
    XCTAssertEqual(channel["resource_id"] as? String, CameraMeshContract.replyResourceID)
    XCTAssertEqual(encodedClock["id"] as? String, CameraMeshContract.clockID)

    let ready = try decodeSnapshotReady(
      Data(
        #"{"version":1,"requestId":"snapshot-1","sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","size":4}"#
          .utf8
      ))
    XCTAssertEqual(ready.requestID, "snapshot-1")
    XCTAssertEqual(ready.size, 4)
    XCTAssertThrowsError(
      try decodeSnapshotReady(
        Data(#"{"version":1,"requestId":"snapshot-1","sha256":"BAD","size":4}"#.utf8)
      ))
    XCTAssertThrowsError(
      try decodeSnapshotReady(
        Data(
          #"{"version":1,"requestId":"snapshot-1","sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","size":4,"extra":true}"#
            .utf8
        )
      ))

    XCTAssertThrowsError(
      try makeSnapshotRequest(
        requestID: "snapshot-2",
        replyPeerID: peerID,
        replyRoutes: [tcp, wss],
        clock: AukiMessageClockReference(peerId: peerID, id: "", hash: clockHash)
      ))
    XCTAssertThrowsError(
      try makeSnapshotRequest(
        requestID: "snapshot-3",
        replyPeerID: peerID,
        replyRoutes: [tcp.replacingOccurrences(of: "/tcp/9000/", with: "/tcp/nope/"), wss],
        clock: AukiMessageClockReference(
          peerId: peerID,
          id: CameraMeshContract.clockID,
          hash: clockHash
        )
      ))
  }

  func testCameraFrameCodecAndJPEGBoundary() throws {
    let jpeg = Data([0xff, 0xd8, 0xff, 0xd9])
    let encoded = try encodeCameraFrameImage(frame: jpeg)
    XCTAssertEqual(encoded, Data([0x12, 0x04, 0xff, 0xd8, 0xff, 0xd9]))
    XCTAssertEqual(try decodeCameraFrameImage(payload: encoded), jpeg)
    XCTAssertNoThrow(try validateJPEG(jpeg))
    XCTAssertThrowsError(try validateJPEG(Data([0xff, 0xd8, 0x00, 0x00])))
  }

  private func infoJSON(peerID: String) -> String {
    #"{"app":"auki-camera-mesh","app_version":"0.1.0","name":"unit camera","session_id":"camera-session","session_clock_id":"camera/utc","session_clock_hash":"\#(clockHash)","session_now_ns":7,"peer_id":"\#(peerID)","app_instance":"native/publisher"}"#
  }

  private func catalogJSON(peerID: String) -> String {
    #"{"resources":[{"variant":"sensor_log","source_peer_id":"\#(peerID)","writer_peer_id":"\#(peerID)","resource_id":"camera/main","state":"live","head":{"kind":"rolling","retention_ns":200000000},"available":{"bytes":0,"entries":0,"duration_ns":0},"sensor":{"kind":"camera","type":"rgb","sensor_id":"camera/main","sensor_hash":"\#(sensorHash)"},"manifest":{"clock":{"peer_id":"\#(peerID)","id":"camera/utc","hash":"\#(clockHash)"},"frame":{"peer_id":"\#(peerID)","id":"camera/optical","hash":"\#(frameHash)"}}},{"variant":"message_channel","owner_peer_id":"\#(peerID)","resource_id":"camera/control","clock":{"peer_id":"\#(peerID)","id":"camera/utc","hash":"\#(clockHash)"}}]}"#
  }

  private func sensorJSON(peerID: String) -> String {
    #"{"peer_id":"\#(peerID)","sensor_id":"camera/main","kind":"camera","type":"rgb","width":480,"height":270,"frame_rate_hz":5,"image_encoding":"jpeg","pixel_format":"rgb8","row_stride_bytes":0,"color_space":"srgb","intrinsics_model":"none","distortion_model":"none","frame":{"peer_id":"\#(peerID)","id":"camera/optical","hash":"\#(frameHash)"}}"#
  }

  private func clockJSON(peerID: String) -> String {
    #"{"peer_id":"\#(peerID)","session_id":"camera-session","clock_id":"camera/utc","type":"utc_clock","unit":"ns","monotonic":false,"epoch":"1970-01-01T00:00:00Z","scope":"global"}"#
  }

  private func frameJSON(peerID: String) -> String {
    #"{"peer_id":"\#(peerID)","frame_id":"camera/optical","handedness":"right","axes":{"x":"right","y":"down","z":"forward"},"units":"meters"}"#
  }

  private func streamManifest(peerID: String) -> AukiStreamManifest {
    AukiStreamManifest(
      sensorId: CameraMeshContract.cameraResourceID,
      sensorHash: sensorHash,
      clockPeerId: peerID,
      clockId: CameraMeshContract.clockID,
      clockHash: clockHash,
      frameId: CameraMeshContract.frameID,
      frameHash: frameHash,
      resourceId: CameraMeshContract.cameraResourceID,
      payload: "camera_frame",
      fromFrameId: "",
      fromFrameHash: "",
      toFrameId: "",
      toFrameHash: "",
      writerMode: "live",
      expectedRateHz: UInt32(CameraMeshContract.rateHz),
      mapPeerId: "",
      mapId: "",
      mapHash: ""
    )
  }
}
