import XCTest
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

    private static func jsonObject(_ json: String) throws -> [String: Any] {
        let object = try JSONSerialization.jsonObject(with: Data(json.utf8))
        return try XCTUnwrap(object as? [String: Any])
    }
}
