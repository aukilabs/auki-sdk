import Foundation
import UIKit

@MainActor
final class CameraStreamerViewModel: ObservableObject {
    @Published var clusterName: String = "ios-camera"
    @Published var discoveryUrl: String = "http://192.168.9.130:8080"
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
    @Published private(set) var statusText = "Stopped"
    @Published private(set) var lastPreviewImage: UIImage?
    @Published private(set) var sessionId = ""
    @Published private(set) var acceptedStreamCount = 0
    @Published private(set) var loggedFrameCount = 0
    @Published private(set) var lastFrameTimestampNs: UInt64?
    @Published private(set) var lastErrorMessage = ""

    var previewImage: UIImage? {
        lastPreviewImage
    }

    private var session: AukiCameraSession?
    private let captureService: CameraCaptureService
    private var startupTask: Task<Void, Never>?
    private var statusPollingTask: Task<Void, Never>?
    private var frameForwardingTasks: [UUID: Task<Void, Never>] = [:]
    private var startupGeneration = UUID()

    init(captureService: CameraCaptureService = CameraCaptureService()) {
        self.captureService = captureService
        self.captureService.delegate = self
    }

    func toggleRunning() async {
        if isRunning || startupTask != nil {
            await stop()
        } else {
            start()
        }
    }

    func handleCapturedFrame(_ frame: CapturedCameraFrame) async {
        await forwardCapturedFrame(frame, generation: startupGeneration)
    }

    private func start() {
        isRunning = true
        peerId = "pending"
        statusText = "Starting"
        lastPreviewImage = nil
        resetSessionStatus()
        let generation = UUID()
        startupGeneration = generation

        startupTask = Task { [clusterName, discoveryUrl, loggingEnabled, streamingEnabled] in
            var startedSession: AukiCameraSession?
            do {
                guard await captureService.requestAccess() else {
                    throw CameraCaptureServiceError.accessDenied
                }
                guard generation == startupGeneration, !Task.isCancelled else {
                    return
                }

                let session = try await AukiCameraSession.start(
                    clusterName: clusterName,
                    discoveryUrl: discoveryUrl,
                    loggingEnabled: loggingEnabled,
                    streamingEnabled: streamingEnabled
                )
                startedSession = session
                guard generation == startupGeneration, !Task.isCancelled else {
                    try? await session.stop()
                    return
                }

                self.session = session
                captureService.setTimestampProvider { session.nowNs() }
                await refreshSessionStatus(session: session, generation: generation)
                try await captureService.start()
                guard generation == startupGeneration, !Task.isCancelled else {
                    await captureService.stop()
                    captureService.setTimestampProvider(nil)
                    try? await session.stop()
                    return
                }
                startupTask = nil
                updateSessionRuntimeOptions()
                peerId = session.peerId
                statusText = "Running"
                startStatusPolling(session: session, generation: generation)
            } catch {
                await captureService.stop()
                captureService.setTimestampProvider(nil)
                try? await startedSession?.stop()
                guard generation == startupGeneration, !Task.isCancelled else {
                    return
                }
                self.session = nil
                startupTask = nil
                peerId = ""
                isRunning = false
                resetSessionStatus()
                lastErrorMessage = error.localizedDescription
                statusText = "Start failed: \(error.localizedDescription)"
            }
        }
    }

    private func stop() async {
        startupGeneration = UUID()
        startupTask?.cancel()
        startupTask = nil
        stopStatusPolling()
        statusText = "Stopping"
        let session = self.session
        self.session = nil
        await captureService.stop()
        captureService.setTimestampProvider(nil)
        await cancelFrameForwardingTasks()

        do {
            try await session?.stop()
            peerId = ""
            isRunning = false
            resetSessionStatus()
            statusText = "Stopped"
        } catch {
            peerId = ""
            isRunning = false
            resetSessionStatus()
            lastErrorMessage = error.localizedDescription
            statusText = "Stop failed: \(error.localizedDescription)"
        }
    }

    private func cancelFrameForwardingTasks() async {
        let tasks = Array(frameForwardingTasks.values)
        for task in tasks {
            task.cancel()
        }
        for task in tasks {
            await task.value
        }
        frameForwardingTasks.removeAll()
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

    private func startStatusPolling(session: AukiCameraSession, generation: UUID) {
        statusPollingTask?.cancel()
        statusPollingTask = Task { [weak self] in
            while !Task.isCancelled {
                await self?.refreshSessionStatus(session: session, generation: generation)
                try? await Task.sleep(nanoseconds: 500_000_000)
            }
        }
    }

    private func stopStatusPolling() {
        statusPollingTask?.cancel()
        statusPollingTask = nil
    }

    private func refreshSessionStatus(session: AukiCameraSession, generation: UUID) async {
        let status = await session.status()
        guard generation == startupGeneration, !Task.isCancelled else {
            return
        }
        sessionId = status.sessionId
        acceptedStreamCount = status.acceptedStreamCount
        loggedFrameCount = status.loggedFrameCount
        lastFrameTimestampNs = status.lastFrameTimestampNs
        lastErrorMessage = status.lastErrorMessage ?? lastErrorMessage
    }

    private func resetSessionStatus() {
        sessionId = ""
        acceptedStreamCount = 0
        loggedFrameCount = 0
        lastFrameTimestampNs = nil
        lastErrorMessage = ""
    }
}

extension CameraStreamerViewModel: CameraCaptureServiceDelegate {
    func cameraCaptureService(_ service: CameraCaptureService, didCapture frame: CapturedCameraFrame) {
        let generation = startupGeneration
        lastPreviewImage = UIImage(data: frame.jpegBytes)
        let taskId = UUID()
        let task = Task { [weak self] in
            await self?.forwardCapturedFrame(frame, generation: generation)
            await MainActor.run {
                self?.frameForwardingTasks[taskId] = nil
            }
        }
        frameForwardingTasks[taskId] = task
    }

    func cameraCaptureService(_ service: CameraCaptureService, didFail error: Error) {
        guard session != nil || startupTask != nil else {
            return
        }
        lastErrorMessage = error.localizedDescription
        statusText = "Capture error: \(error.localizedDescription)"
    }

    private func forwardCapturedFrame(_ frame: CapturedCameraFrame, generation: UUID) async {
        guard generation == startupGeneration, !Task.isCancelled, let session else {
            return
        }

        do {
            try Task.checkCancellation()
            try await session.handleCapturedFrame(frame)
            await refreshSessionStatus(session: session, generation: generation)
        } catch is CancellationError {
            return
        } catch {
            guard generation == startupGeneration, !Task.isCancelled else {
                return
            }
            await refreshSessionStatus(session: session, generation: generation)
            lastErrorMessage = error.localizedDescription
            statusText = "Frame error: \(error.localizedDescription)"
        }
    }
}
