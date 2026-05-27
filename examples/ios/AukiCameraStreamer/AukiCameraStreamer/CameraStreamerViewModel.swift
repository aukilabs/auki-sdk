import Foundation

@MainActor
final class CameraStreamerViewModel: ObservableObject {
    @Published var clusterName = "auki-camera"
    @Published var discoveryUrl = "http://127.0.0.1:8091"
    @Published var loggingEnabled = true {
        didSet {
            updateSessionRuntimeOptions()
        }
    }
    @Published var streamingEnabled = true {
        didSet {
            updateSessionRuntimeOptions()
        }
    }
    @Published private(set) var peerId = ""
    @Published private(set) var isRunning = false
    @Published private(set) var statusText = "Idle"

    private var session: AukiCameraSession?
    private var startupTask: Task<Void, Never>?
    private var startupGeneration = UUID()

    func toggleRunning() async {
        if isRunning || startupTask != nil {
            await stop()
        } else {
            start()
        }
    }

    func handleCapturedFrame(_ frame: CapturedCameraFrame) async {
        guard let session else {
            return
        }

        do {
            try await session.handleCapturedFrame(frame)
        } catch {
            statusText = "Frame error: \(error.localizedDescription)"
        }
    }

    private func start() {
        isRunning = true
        peerId = "pending"
        statusText = "Starting"
        let generation = UUID()
        startupGeneration = generation

        startupTask = Task { [clusterName, discoveryUrl, loggingEnabled, streamingEnabled] in
            do {
                let session = try await AukiCameraSession.start(
                    clusterName: clusterName,
                    discoveryUrl: discoveryUrl,
                    loggingEnabled: loggingEnabled,
                    streamingEnabled: streamingEnabled
                )
                guard generation == startupGeneration, !Task.isCancelled else {
                    try? await session.stop()
                    return
                }

                self.session = session
                startupTask = nil
                updateSessionRuntimeOptions()
                peerId = session.peerId
                statusText = "Running"
            } catch {
                guard generation == startupGeneration, !Task.isCancelled else {
                    return
                }
                self.session = nil
                startupTask = nil
                peerId = ""
                isRunning = false
                statusText = "Start failed: \(error.localizedDescription)"
            }
        }
    }

    private func stop() async {
        startupGeneration = UUID()
        startupTask?.cancel()
        startupTask = nil
        statusText = "Stopping"
        let session = self.session
        self.session = nil

        do {
            try await session?.stop()
            peerId = ""
            isRunning = false
            statusText = "Stopped"
        } catch {
            peerId = ""
            isRunning = false
            statusText = "Stop failed: \(error.localizedDescription)"
        }
    }

    private func updateSessionRuntimeOptions() {
        guard let session else {
            return
        }

        let loggingEnabled = loggingEnabled
        let streamingEnabled = streamingEnabled
        Task {
            await session.setRuntimeOptions(
                loggingEnabled: loggingEnabled,
                streamingEnabled: streamingEnabled
            )
        }
    }
}
