import XCTest
@testable import AukiNetworkSignaledWebRTC
import auki_network
@testable import AukiCameraStreamer

final class AukiCameraModelsTests: XCTestCase {
    @MainActor
    func testViewModelExposesRequiredInitialOperatorState() {
        let viewModel = CameraStreamerViewModel()

        XCTAssertEqual(viewModel.clusterName, "ios-camera")
        XCTAssertEqual(viewModel.discoveryUrl, "http://192.168.9.130:8080")
        XCTAssertTrue(viewModel.loggingEnabled)
        XCTAssertTrue(viewModel.streamingEnabled)
        XCTAssertEqual(viewModel.peerId, "")
        XCTAssertFalse(viewModel.isRunning)
        XCTAssertEqual(viewModel.statusText, "Stopped")
        XCTAssertNil(viewModel.lastPreviewImage)
    }

    func testDescriptorUsesPeerAndSessionStableIds() {
        let descriptor = AukiCameraDefaults.descriptor(peerId: "peer-a", sessionId: "session-b")
        XCTAssertEqual(descriptor.sensorId, "peer-a/session-b/camera")
        XCTAssertEqual(descriptor.frameId, "peer-a/session-b/camera_optical")
    }

    func testSessionCatalogsUseStableCameraSensorAndResourceIds() throws {
        let descriptor = AukiCameraDefaults.descriptor(peerId: "peer-a", sessionId: "session-b")
        let catalog = AukiCameraSessionCatalog(
            descriptor: descriptor,
            sensorHash: "sensor-hash",
            frameHash: "frame-hash",
            clockId: "clock-id",
            clockHash: "clock-hash"
        )

        let sensorCatalog = try Self.jsonObject(catalog.sensorCatalogJson())
        let sensors = try XCTUnwrap(sensorCatalog["sensors"] as? [[String: Any]])
        XCTAssertEqual(sensors.first?["sensor_id"] as? String, "peer-a/session-b/camera")
        XCTAssertEqual(sensors.first?["kind"] as? String, "camera")

        let resourceCatalog = try Self.jsonObject(catalog.resourceCatalogJson())
        let resources = try XCTUnwrap(resourceCatalog["resources"] as? [[String: Any]])
        XCTAssertEqual(resources.first?["kind"] as? String, "sensor_stream")
        XCTAssertEqual(resources.first?["sensor_id"] as? String, "peer-a/session-b/camera")
    }

    func testSessionLogRootIsScopedBySessionId() throws {
        let fileManager = FileManager.default
        let appRoot = fileManager.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        try fileManager.createDirectory(at: appRoot, withIntermediateDirectories: true)
        defer {
            try? fileManager.removeItem(at: appRoot)
        }

        let logRoot = try AukiCameraSession.makeLogRoot(
            appRoot: appRoot.path,
            sessionId: "session-b",
            fileManager: fileManager
        )

        XCTAssertTrue(logRoot.hasSuffix("/sensor-logs/session-b/camera"))
        XCTAssertTrue(fileManager.fileExists(atPath: logRoot))
    }

    func testSessionUsesDiscoverySignaledWebRtcTransport() {
        XCTAssertEqual(AukiCameraSession.transportKind, "discovery-signaled-webrtc")
        XCTAssertEqual(AukiCameraSession.listenAddrs, [])
    }

    func testSignaledWebRtcFramedRequestTimesOutWhenPeerNeverAnswers() async throws {
        let discovery = NoAnswerDiscovery()
        let peer = try AukiSignaledWebRTCPeer(
            localPeerId: "peer-a",
            discoveryUrl: "http://discovery.local",
            discovery: discovery,
            operationTimeoutNanoseconds: 200_000_000
        )
        peer.start()

        do {
            let remote = try AukiSignaledWebRTCPeer.formatSignaledAddress(
                discoveryUrl: "http://discovery.local",
                peerId: "peer-b"
            )
            _ = try await peer.requestFramed(
                peerMultiaddr: remote,
                protocolId: "/auki/join/0.0.1",
                payload: Data("{}".utf8)
            )
            XCTFail("requestFramed unexpectedly completed")
        } catch let error as AukiSignaledWebRTCError {
            guard case .timedOut = error else {
                XCTFail("expected timedOut, got \(error)")
                await peer.stop()
                return
            }
        } catch {
            XCTFail("expected AukiSignaledWebRTCError.timedOut, got \(error)")
        }

        await peer.stop()
        XCTAssertGreaterThan(discovery.sentSignalCount, 0)
    }

    func testSignaledWebRtcFramedRequestCompletesBetweenTwoPeers() async throws {
        let discovery = InMemorySignalDiscovery()
        let peerA = try AukiSignaledWebRTCPeer(
            localPeerId: "peer-a",
            discoveryUrl: "http://discovery.local",
            discovery: discovery,
            operationTimeoutNanoseconds: 5_000_000_000
        )
        let peerB = try AukiSignaledWebRTCPeer(
            localPeerId: "peer-b",
            discoveryUrl: "http://discovery.local",
            discovery: discovery,
            operationTimeoutNanoseconds: 5_000_000_000
        )
        peerB.handleFramed("/auki/join/0.0.1") { payload, _ in
            let request = try Self.jsonObject(String(decoding: payload, as: UTF8.self))
            guard request["peer_id"] as? String == "peer-a" else {
                return Data(#"{"kind":"reject","reason":"wrong peer"}"#.utf8)
            }
            return Data(#"{"kind":"accept"}"#.utf8)
        }
        peerA.start()
        peerB.start()

        do {
            let response = try await peerA.requestFramed(
                peerMultiaddr: peerB.signaledMultiaddr,
                protocolId: "/auki/join/0.0.1",
                payload: Data(#"{"peer_id":"peer-a"}"#.utf8)
            )
            let responseJson = try Self.jsonObject(String(decoding: response, as: UTF8.self))
            XCTAssertEqual(responseJson["kind"] as? String, "accept")
        } catch {
            await peerA.stop()
            await peerB.stop()
            throw error
        }

        await peerA.stop()
        await peerB.stop()
    }

    func testSignaledWebRtcLengthPrefixedFramesAreChunkedForDataChannelTransport() throws {
        let payload = Data((0..<40_000).map { UInt8($0 % 251) })

        let chunks = AukiWebRTCDataChannelStream.lengthPrefixedChunks(
            payload,
            maxChunkByteCount: 16_384
        )

        XCTAssertGreaterThan(chunks.count, 1)
        XCTAssertTrue(chunks.allSatisfy { $0.count <= 16_384 })

        let encoded = chunks.reduce(into: Data()) { output, chunk in
            output.append(chunk)
        }
        let length = encoded.prefix(4).withUnsafeBytes { pointer in
            UInt32(bigEndian: pointer.load(as: UInt32.self))
        }
        XCTAssertEqual(length, UInt32(payload.count))
        XCTAssertEqual(encoded.dropFirst(4), payload[...])
    }

    private static func jsonObject(_ json: String) throws -> [String: Any] {
        let object = try JSONSerialization.jsonObject(with: Data(json.utf8))
        return try XCTUnwrap(object as? [String: Any])
    }
}

private final class NoAnswerDiscovery: AukiDiscoveryClientProtocol, @unchecked Sendable {
    private let lock = NSLock()
    private var signalsSent = 0

    var sentSignalCount: Int {
        lock.lock()
        defer { lock.unlock() }
        return signalsSent
    }

    func discoverNodesJson(queryJson: String, timeoutMs: UInt64) throws -> String {
        #"{"nodes":[]}"#
    }

    func discoverPeersJson(queryJson: String, timeoutMs: UInt64) throws -> String {
        #"{"clusters":[]}"#
    }

    func pollSignalsJson(query: BindingSignalPoll, timeoutMs: UInt64) throws -> String {
        Thread.sleep(forTimeInterval: 1)
        return #"{"messages":[]}"#
    }

    func registerPeerJson(registrationJson: String, timeoutMs: UInt64) throws -> String {
        #"{"kind":"already_exists"}"#
    }

    func sendSignalJson(request: BindingSignalRequest, timeoutMs: UInt64) throws -> String {
        lock.lock()
        signalsSent += 1
        lock.unlock()
        return #"{"id":1,"recipient_peer_id":"peer-b","from_peer_id":"peer-a","connection_id":"conn","kind":"offer","payload":{},"created_ns":1}"#
    }

    func unregisterPeerJson(peerId: String, timeoutMs: UInt64) throws {}
}

private final class InMemorySignalDiscovery: AukiDiscoveryClientProtocol, @unchecked Sendable {
    private let condition = NSCondition()
    private var nextId: UInt64 = 1
    private var messages: [StoredSignal] = []

    func discoverNodesJson(queryJson: String, timeoutMs: UInt64) throws -> String {
        #"{"nodes":[]}"#
    }

    func discoverPeersJson(queryJson: String, timeoutMs: UInt64) throws -> String {
        #"{"clusters":[]}"#
    }

    func pollSignalsJson(query: BindingSignalPoll, timeoutMs: UInt64) throws -> String {
        let waitMs = min(timeoutMs, 100)
        let deadline = Date().addingTimeInterval(TimeInterval(waitMs) / 1_000)
        condition.lock()
        defer { condition.unlock() }
        while true {
            let available = messages.filter { message in
                message.recipientPeerId == query.peerId && message.id > query.since
            }
            if !available.isEmpty || Date() >= deadline {
                return #"{"messages":[\#(available.map(\.json).joined(separator: ","))]}"#
            }
            condition.wait(until: deadline)
        }
    }

    func registerPeerJson(registrationJson: String, timeoutMs: UInt64) throws -> String {
        #"{"kind":"already_exists"}"#
    }

    func sendSignalJson(request: BindingSignalRequest, timeoutMs: UInt64) throws -> String {
        condition.lock()
        let id = nextId
        nextId += 1
        let message = StoredSignal(
            id: id,
            recipientPeerId: request.recipientPeerId,
            fromPeerId: request.fromPeerId,
            connectionId: request.connectionId,
            kind: request.kind,
            payloadJson: request.payloadJson
        )
        messages.append(message)
        condition.broadcast()
        condition.unlock()
        return message.json
    }

    func unregisterPeerJson(peerId: String, timeoutMs: UInt64) throws {}
}

private struct StoredSignal {
    let id: UInt64
    let recipientPeerId: String
    let fromPeerId: String
    let connectionId: String
    let kind: String
    let payloadJson: String

    var json: String {
        #"{"id":\#(id),"recipient_peer_id":\#(Self.jsonString(recipientPeerId)),"from_peer_id":\#(Self.jsonString(fromPeerId)),"connection_id":\#(Self.jsonString(connectionId)),"kind":\#(Self.jsonString(kind)),"payload":\#(payloadJson),"created_ns":\#(id)}"#
    }

    private static func jsonString(_ value: String) -> String {
        let data = try? JSONEncoder().encode(value)
        return data.map { String(decoding: $0, as: UTF8.self) } ?? #""""#
    }
}
