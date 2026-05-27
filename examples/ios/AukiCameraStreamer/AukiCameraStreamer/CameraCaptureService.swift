import AVFoundation
import CoreImage
import Foundation

@MainActor
protocol CameraCaptureServiceDelegate: AnyObject {
    func cameraCaptureService(_ service: CameraCaptureService, didCapture frame: CapturedCameraFrame)
    func cameraCaptureService(_ service: CameraCaptureService, didFail error: Error)
}

protocol CameraCaptureControlling {
    func requestAccess() async -> Bool
    func start() async throws
    func stop() async
}

enum CameraCaptureServiceError: LocalizedError {
    case accessDenied
    case cameraUnavailable
    case cannotAddInput
    case cannotAddOutput
    case jpegEncodingFailed
    case missingTimestampProvider

    var errorDescription: String? {
        switch self {
        case .accessDenied:
            return "Camera access was denied."
        case .cameraUnavailable:
            return "The back wide-angle camera is unavailable."
        case .cannotAddInput:
            return "The camera input could not be added to the capture session."
        case .cannotAddOutput:
            return "The camera frame output could not be added to the capture session."
        case .jpegEncodingFailed:
            return "The captured camera frame could not be encoded as JPEG."
        case .missingTimestampProvider:
            return "The SDK session clock is not available."
        }
    }
}

final class CameraCaptureService: NSObject, CameraCaptureControlling, @unchecked Sendable {
    weak var delegate: CameraCaptureServiceDelegate?

    private let session = AVCaptureSession()
    private let output = AVCaptureVideoDataOutput()
    private let captureQueue = DispatchQueue(label: "AukiCameraStreamer.capture")
    private let ciContext = CIContext()
    private let colorSpace = CGColorSpace(name: CGColorSpace.sRGB)!
    private let timestampProviderLock = NSLock()
    private let minimumFrameIntervalNs: UInt64

    private var timestampProvider: (@Sendable () -> UInt64)?
    private var isConfigured = false
    private var lastFrameTimestampNs: UInt64 = 0

    init(minimumFrameIntervalNs: UInt64 = 100_000_000) {
        self.minimumFrameIntervalNs = minimumFrameIntervalNs
        super.init()
    }

    func setTimestampProvider(_ provider: (@Sendable () -> UInt64)?) {
        timestampProviderLock.lock()
        timestampProvider = provider
        timestampProviderLock.unlock()
    }

    func requestAccess() async -> Bool {
        switch AVCaptureDevice.authorizationStatus(for: .video) {
        case .authorized:
            return true
        case .notDetermined:
            return await withCheckedContinuation { continuation in
                AVCaptureDevice.requestAccess(for: .video) { granted in
                    continuation.resume(returning: granted)
                }
            }
        case .denied, .restricted:
            return false
        @unknown default:
            return false
        }
    }

    func start() async throws {
        guard await requestAccess() else {
            throw CameraCaptureServiceError.accessDenied
        }
        guard currentTimestampProvider() != nil else {
            throw CameraCaptureServiceError.missingTimestampProvider
        }

        try await withCheckedThrowingContinuation { continuation in
            captureQueue.async { [self] in
                do {
                    try configureSessionIfNeeded()
                    if !session.isRunning {
                        session.startRunning()
                    }
                    continuation.resume()
                } catch {
                    continuation.resume(throwing: error)
                }
            }
        }
    }

    func stop() async {
        await withCheckedContinuation { continuation in
            captureQueue.async { [self] in
                if session.isRunning {
                    session.stopRunning()
                }
                lastFrameTimestampNs = 0
                continuation.resume()
            }
        }
    }

    private func currentTimestampProvider() -> (@Sendable () -> UInt64)? {
        timestampProviderLock.lock()
        let provider = timestampProvider
        timestampProviderLock.unlock()
        return provider
    }

    private func configureSessionIfNeeded() throws {
        guard !isConfigured else {
            return
        }

        guard let camera = AVCaptureDevice.default(
            .builtInWideAngleCamera,
            for: .video,
            position: .back
        ) else {
            throw CameraCaptureServiceError.cameraUnavailable
        }

        let input = try AVCaptureDeviceInput(device: camera)
        session.beginConfiguration()
        session.sessionPreset = .high

        guard session.canAddInput(input) else {
            session.commitConfiguration()
            throw CameraCaptureServiceError.cannotAddInput
        }
        session.addInput(input)

        output.alwaysDiscardsLateVideoFrames = true
        output.videoSettings = [
            kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA
        ]
        output.setSampleBufferDelegate(self, queue: captureQueue)

        guard session.canAddOutput(output) else {
            output.setSampleBufferDelegate(nil, queue: nil)
            session.removeInput(input)
            session.commitConfiguration()
            throw CameraCaptureServiceError.cannotAddOutput
        }
        session.addOutput(output)
        session.commitConfiguration()
        isConfigured = true
    }

    private func capturedFrame(from sampleBuffer: CMSampleBuffer) throws -> CapturedCameraFrame? {
        guard let timestampNs = currentTimestampProvider()?() else {
            throw CameraCaptureServiceError.missingTimestampProvider
        }
        guard timestampNs >= lastFrameTimestampNs + minimumFrameIntervalNs || lastFrameTimestampNs == 0 else {
            return nil
        }
        guard let pixelBuffer = CMSampleBufferGetImageBuffer(sampleBuffer) else {
            return nil
        }

        let image = CIImage(cvPixelBuffer: pixelBuffer)
        guard let jpegBytes = ciContext.jpegRepresentation(
            of: image,
            colorSpace: colorSpace,
            options: [:]
        ) else {
            throw CameraCaptureServiceError.jpegEncodingFailed
        }

        lastFrameTimestampNs = timestampNs
        return CapturedCameraFrame(
            jpegBytes: jpegBytes,
            timestampNs: timestampNs,
            width: CVPixelBufferGetWidth(pixelBuffer),
            height: CVPixelBufferGetHeight(pixelBuffer)
        )
    }
}

extension CameraCaptureService: AVCaptureVideoDataOutputSampleBufferDelegate {
    func captureOutput(
        _ output: AVCaptureOutput,
        didOutput sampleBuffer: CMSampleBuffer,
        from connection: AVCaptureConnection
    ) {
        do {
            guard let frame = try capturedFrame(from: sampleBuffer) else {
                return
            }
            Task { @MainActor [weak self] in
                guard let self else {
                    return
                }
                delegate?.cameraCaptureService(self, didCapture: frame)
            }
        } catch {
            Task { @MainActor [weak self] in
                guard let self else {
                    return
                }
                delegate?.cameraCaptureService(self, didFail: error)
            }
        }
    }
}
