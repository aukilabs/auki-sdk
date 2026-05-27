import Foundation
import auki_domain
import auki_logs
import auki_manifests
import auki_network
import auki_registry
import auki_time

enum AukiCameraSessionError: Error {
    case missingApplicationSupportDirectory
    case invalidStreamOpenRequest
    case timestampOutOfRange(UInt64)
}

struct AukiCameraSessionCatalog: Equatable {
    let descriptor: CameraSensorDescriptor
    let sensorHash: String
    let frameHash: String
    let clockId: String
    let clockHash: String

    var streamResourceId: String {
        "\(descriptor.sensorId)/stream"
    }

    func sensorCatalogJson() throws -> String {
        try Self.jsonString([
            "sensors": [[
                "sensor_id": descriptor.sensorId,
                "sensor_hash": sensorHash,
                "kind": "camera"
            ]]
        ])
    }

    func resourceCatalogJson() throws -> String {
        try Self.jsonString([
            "resources": [[
                "id": streamResourceId,
                "kind": "sensor_stream",
                "sensor_id": descriptor.sensorId,
                "sensor_hash": sensorHash,
                "sensor_kind": "camera",
                "stream_protocol": "/auki/stream/0.1.0",
                "payload": "camera_frame"
            ]]
        ])
    }

    func streamManifestJson() throws -> String {
        try Self.jsonString([
            "sensor_id": descriptor.sensorId,
            "sensor_hash": sensorHash,
            "clock_id": clockId,
            "clock_hash": clockHash,
            "frame_id": descriptor.frameId,
            "frame_hash": frameHash
        ])
    }

    static func sensorEntryJson(
        descriptor: CameraSensorDescriptor,
        frameHash: String,
        width: Int = 1_920,
        height: Int = 1_080,
        frameRateHz: Int = 30
    ) throws -> String {
        try jsonString([
            "sensor_id": descriptor.sensorId,
            "type": "camera",
            "width": width,
            "height": height,
            "frame_rate_hz": frameRateHz,
            "frame_id": descriptor.frameId,
            "frame_hash": frameHash,
            "pixel_format": "jpeg",
            "color_space": "srgb",
            "intrinsics_model": "pinhole",
            "distortion_model": "unknown"
        ])
    }

    static func registryEntriesJson(entries: [RegistryEntry]) throws -> String {
        try jsonString([
            "entries": entries.map { entry in
                [
                    "kind": entry.kind,
                    "id": entry.id,
                    "hash": entry.hash,
                    "canonical_json": entry.canonicalJson
                ]
            }
        ])
    }

    static func jsonString(_ object: Any) throws -> String {
        let data = try JSONSerialization.data(withJSONObject: object, options: [.sortedKeys])
        return String(decoding: data, as: UTF8.self)
    }

    struct RegistryEntry: Equatable {
        let kind: String
        let id: String
        let hash: String
        let canonicalJson: String
    }
}

actor AukiCameraSession {
    private static let agentVersion = "ios-camera-streamer/0.1.0"
    private static let streamPollIntervalNs: UInt64 = 100_000_000

    let peerId: String
    let sessionId: String
    let appRoot: String
    let logRoot: String

    private let descriptor: CameraSensorDescriptor
    private let catalog: AukiCameraSessionCatalog
    private let clock: SessionClock
    private let manager: DomainClusterManager
    private let log: BytesLog
    private let fanout: CameraStreamFanout
    private var loggingEnabled: Bool
    private var streamingEnabled: Bool
    private var streamPollTask: Task<Void, Never>?

    private init(
        peerId: String,
        sessionId: String,
        appRoot: String,
        logRoot: String,
        descriptor: CameraSensorDescriptor,
        catalog: AukiCameraSessionCatalog,
        clock: SessionClock,
        manager: DomainClusterManager,
        log: BytesLog,
        fanout: CameraStreamFanout,
        loggingEnabled: Bool,
        streamingEnabled: Bool
    ) {
        self.peerId = peerId
        self.sessionId = sessionId
        self.appRoot = appRoot
        self.logRoot = logRoot
        self.descriptor = descriptor
        self.catalog = catalog
        self.clock = clock
        self.manager = manager
        self.log = log
        self.fanout = fanout
        self.loggingEnabled = loggingEnabled
        self.streamingEnabled = streamingEnabled
    }

    static func start(
        clusterName: String,
        discoveryUrl: String,
        loggingEnabled: Bool,
        streamingEnabled: Bool,
        seedStore: KeychainSeedStore = KeychainSeedStore(),
        fileManager: FileManager = .default
    ) async throws -> AukiCameraSession {
        let walletSeed = try seedStore.loadOrCreateSeed()
        let peerId = try peerIdFromWalletSeed(seed: walletSeed)
        let sessionId = UUID().uuidString.lowercased()
        let descriptor = AukiCameraDefaults.descriptor(peerId: peerId, sessionId: sessionId)
        let clock = SessionClock(peerId: peerId, sessionId: sessionId, name: AukiCameraDefaults.sensorName)
        let appRoot = try makeAppRoot(fileManager: fileManager)
        let logRoot = try makeLogRoot(appRoot: appRoot, sessionId: sessionId, fileManager: fileManager)

        let frameJson = frameRosOpticalJson(frameId: descriptor.frameId)
        let frameOutcome = try writeFrameEntryJson(appRoot: appRoot, entryJson: frameJson)
        let sensorJson = try AukiCameraSessionCatalog.sensorEntryJson(
            descriptor: descriptor,
            frameHash: frameOutcome.hash
        )
        let sensorCanonicalJson = try sensorEntryCanonicalJson(entryJson: sensorJson)
        let sensorOutcome = try writeSensorEntryJson(appRoot: appRoot, entryJson: sensorCanonicalJson)
        let clockJson = try clock.registryEntryJson()
        let clockOutcome = try writeClockEntryJson(appRoot: appRoot, entryJson: clockJson)

        let catalog = AukiCameraSessionCatalog(
            descriptor: descriptor,
            sensorHash: sensorOutcome.hash,
            frameHash: frameOutcome.hash,
            clockId: clock.clockId(),
            clockHash: clock.clockHash()
        )

        let manifestJson = buildSensorLogManifestJson(
            appId: "auki-camera-streamer",
            sessionId: sessionId,
            sensorId: descriptor.sensorId,
            sensorHash: sensorOutcome.hash,
            clockId: clock.clockId(),
            clockHash: clock.clockHash(),
            frameId: descriptor.frameId,
            frameHash: frameOutcome.hash,
            segmentDurationNs: AukiCameraDefaults.segmentDurationNs,
            retentionNs: AukiCameraDefaults.retentionNs
        )
        let log = try BytesLog.open(root: logRoot, manifestJson: manifestJson)
        try log.setRetention(retentionNs: Int64(AukiCameraDefaults.retentionNs))

        let manager = try await bootstrapDomainClusterManagerAutoAdvertise(
            targetMode: .joinOrCreate,
            targetName: clusterName,
            walletSeed: walletSeed,
            listenAddrs: ["/ip4/0.0.0.0/udp/0/webrtc-direct"],
            advertiseMultiaddrsOverride: [],
            advertiseResolutionMs: 5_000,
            discoveryUrl: discoveryUrl,
            daemonInfo: DaemonInfo(
                app: "auki-camera-streamer",
                name: "ios-camera",
                sessionId: sessionId,
                sessionClockId: clock.clockId(),
                sessionClockHash: clock.clockHash(),
                appInstance: UUID().uuidString.replacingOccurrences(of: "-", with: "").lowercased()
            ),
            agentVersion: agentVersion
        )

        try manager.setStaticSensorCatalogJson(catalogJson: catalog.sensorCatalogJson())
        try manager.setStaticResourceCatalogJson(catalogJson: catalog.resourceCatalogJson())
        try manager.setStaticRegistryEntriesJson(entriesJson: AukiCameraSessionCatalog.registryEntriesJson(entries: [
            .init(kind: "clock", id: clock.clockId(), hash: clockOutcome.hash, canonicalJson: clockJson),
            .init(kind: "frame", id: descriptor.frameId, hash: frameOutcome.hash, canonicalJson: frameJson),
            .init(kind: "sensor", id: descriptor.sensorId, hash: sensorOutcome.hash, canonicalJson: sensorCanonicalJson)
        ]))

        let fanout = CameraStreamFanout(sink: DomainCameraStreamSink(manager: manager))
        let session = AukiCameraSession(
            peerId: manager.localPeerId(),
            sessionId: sessionId,
            appRoot: appRoot,
            logRoot: logRoot,
            descriptor: descriptor,
            catalog: catalog,
            clock: clock,
            manager: manager,
            log: log,
            fanout: fanout,
            loggingEnabled: loggingEnabled,
            streamingEnabled: streamingEnabled
        )
        await session.startPollingStreamRequests()
        return session
    }

    func handleCapturedFrame(_ frame: CapturedCameraFrame) async throws {
        let payload = try CameraFrameCodec.encode(jpegBytes: frame.jpegBytes)
        if loggingEnabled {
            guard frame.timestampNs <= UInt64(Int64.max) else {
                throw AukiCameraSessionError.timestampOutOfRange(frame.timestampNs)
            }
            try log.append(timestampNs: Int64(frame.timestampNs), payload: payload)
        }
        if streamingEnabled {
            try await fanout.pushEncodedPayload(timestampNs: frame.timestampNs, payload: payload)
        }
    }

    func setRuntimeOptions(loggingEnabled: Bool, streamingEnabled: Bool) {
        self.loggingEnabled = loggingEnabled
        self.streamingEnabled = streamingEnabled
    }

    func stop() async throws {
        streamPollTask?.cancel()
        streamPollTask = nil
        var firstError: Error?

        do {
            try await fanout.finishAll()
        } catch {
            firstError = error
        }

        do {
            try log.flush()
        } catch {
            if firstError == nil {
                firstError = error
            }
        }

        do {
            try await manager.shutdown()
        } catch {
            if firstError == nil {
                firstError = error
            }
        }

        if let firstError {
            throw firstError
        }
    }

    private func startPollingStreamRequests() {
        streamPollTask = Task { [weak self] in
            while !Task.isCancelled {
                await self?.drainStreamOpenRequests()
                try? await Task.sleep(nanoseconds: Self.streamPollIntervalNs)
            }
        }
    }

    private func drainStreamOpenRequests() async {
        let events = manager.drainStreamOpenRequests(maxEvents: 32)
        for event in events where event.kind == "stream_open_request" {
            guard let responderId = event.responderId else {
                continue
            }

            do {
                let sensorId = try sensorId(from: event.payloadJson)
                guard sensorId == descriptor.sensorId else {
                    try manager.declineStreamOpen(responderId: responderId, reason: "sensor_not_found")
                    continue
                }
                guard streamingEnabled else {
                    try manager.declineStreamOpen(responderId: responderId, reason: "sensor_unavailable")
                    continue
                }

                let streamId = try manager.acceptStreamOpen(
                    responderId: responderId,
                    manifestJson: catalog.streamManifestJson()
                )
                await fanout.accept(streamId: String(streamId))
            } catch {
                try? manager.declineStreamOpen(responderId: responderId, reason: "sensor_unavailable")
            }
        }
    }

    private func sensorId(from payloadJson: String) throws -> String {
        let object = try JSONSerialization.jsonObject(with: Data(payloadJson.utf8))
        guard
            let dictionary = object as? [String: Any],
            let sensorId = dictionary["sensor_id"] as? String
        else {
            throw AukiCameraSessionError.invalidStreamOpenRequest
        }
        return sensorId
    }

    private static func makeAppRoot(fileManager: FileManager) throws -> String {
        guard let applicationSupport = fileManager.urls(
            for: .applicationSupportDirectory,
            in: .userDomainMask
        ).first else {
            throw AukiCameraSessionError.missingApplicationSupportDirectory
        }
        let root = applicationSupport.appendingPathComponent("AukiCameraStreamer", isDirectory: true)
        try fileManager.createDirectory(at: root, withIntermediateDirectories: true)
        return root.path
    }

    static func makeLogRoot(appRoot: String, sessionId: String, fileManager: FileManager) throws -> String {
        let root = URL(fileURLWithPath: appRoot, isDirectory: true)
            .appendingPathComponent("sensor-logs", isDirectory: true)
            .appendingPathComponent(sessionId, isDirectory: true)
            .appendingPathComponent("camera", isDirectory: true)
        try fileManager.createDirectory(at: root, withIntermediateDirectories: true)
        return root.path
    }
}
