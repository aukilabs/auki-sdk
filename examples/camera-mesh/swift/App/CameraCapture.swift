import AVFoundation
import AukiCameraMesh
import CoreImage
import CoreVideo
import Foundation
import ImageIO

enum CameraCaptureError: LocalizedError, Sendable {
  case permissionDenied
  case unavailable
  case cannotAddInput
  case cannotAddOutput
  case unsupportedLandscapeRotation
  case failedToStart
  case alreadyStopped
  case interrupted(String)
  case runtimeFailure(String)
  case stoppedUnexpectedly
  case frameStalled

  var errorDescription: String? {
    switch self {
    case .permissionDenied:
      "Camera access was denied. Enable it in Settings to publish."
    case .unavailable:
      "No back camera is available."
    case .cannotAddInput:
      "The back camera could not be added to the capture session."
    case .cannotAddOutput:
      "Video frames could not be added to the capture session."
    case .unsupportedLandscapeRotation:
      "The back camera cannot produce the fixed landscape Camera Mesh feed."
    case .failedToStart:
      "The camera capture session did not start."
    case .alreadyStopped:
      "This camera capture has already stopped. Create a new capture to restart."
    case .interrupted(let reason):
      "Camera capture was interrupted: \(reason)"
    case .runtimeFailure(let reason):
      "Camera capture failed: \(reason)"
    case .stoppedUnexpectedly:
      "Camera capture stopped unexpectedly."
    case .frameStalled:
      "Camera capture did not produce an encoded JPEG for 3 seconds."
    }
  }
}

/// A foreground, single-use source of bounded Camera Mesh JPEG frames.
///
/// Session state and `AVCaptureVideoDataOutput` callbacks share one serial
/// queue. The stream retains only its newest frame, so a slow consumer cannot
/// build an unbounded queue of JPEGs.
final class CameraCapture: NSObject, @unchecked Sendable {
  let frames: AsyncThrowingStream<Data, any Error>

  private enum State: Equatable {
    case idle
    case running
    case stopped
  }

  private static let targetWidth = CGFloat(CameraMeshContract.width)
  private static let targetHeight = CGFloat(CameraMeshContract.height)
  private static let frameIntervalNanoseconds =
    UInt64(1_000_000_000 / CameraMeshContract.rateHz)
  private static let watchdogTimeoutNanoseconds = UInt64(3_000_000_000)
  private let frameContinuation: AsyncThrowingStream<Data, any Error>.Continuation
  private let captureQueue = DispatchQueue(
    label: "com.aukilabs.examples.camera-mesh.capture",
    qos: .userInitiated
  )
  private let session = AVCaptureSession()
  private let videoOutput = AVCaptureVideoDataOutput()
  private let imageContext = CIContext(options: [.cacheIntermediates: false])
  private let colorSpace = CGColorSpace(name: CGColorSpace.sRGB)!

  // Accessed only on captureQueue.
  private var state = State.idle
  private var configured = false
  private var lastFrameUptimeNanoseconds: UInt64?
  private var watchdogStartedUptimeNanoseconds: UInt64?
  private var lastSuccessfulJPEGUptimeNanoseconds: UInt64?
  private var frameWatchdog: DispatchSourceTimer?
  private var notificationObservers: [NSObjectProtocol] = []

  override init() {
    let pair = AsyncThrowingStream<Data, any Error>.makeStream(
      bufferingPolicy: .bufferingNewest(1)
    )
    frames = pair.stream
    frameContinuation = pair.continuation
    super.init()

    frameContinuation.onTermination = { [weak self] _ in
      guard let self else { return }
      captureQueue.async { [weak self] in
        self?.stopOnCaptureQueue()
      }
    }
  }

  /// Requests camera access, configures the back camera, and starts capture.
  /// Repeated calls while running are harmless; a stopped instance is final.
  func start() async throws {
    guard await Self.requestCameraAccess() else {
      await stop()
      throw CameraCaptureError.permissionDenied
    }

    try await withCheckedThrowingContinuation {
      (continuation: CheckedContinuation<Void, any Error>) in
      captureQueue.async { [self] in
        do {
          try startOnCaptureQueue()
          continuation.resume()
        } catch {
          stopOnCaptureQueue()
          continuation.resume(throwing: error)
        }
      }
    }
  }

  /// Stops capture and finishes `frames`. Safe to call more than once.
  func stop() async {
    await withCheckedContinuation { continuation in
      captureQueue.async { [self] in
        stopOnCaptureQueue()
        continuation.resume()
      }
    }
  }

  private static func requestCameraAccess() async -> Bool {
    switch AVCaptureDevice.authorizationStatus(for: .video) {
    case .authorized:
      true
    case .notDetermined:
      await AVCaptureDevice.requestAccess(for: .video)
    case .denied, .restricted:
      false
    @unknown default:
      false
    }
  }

  private func startOnCaptureQueue() throws {
    switch state {
    case .running:
      return
    case .stopped:
      throw CameraCaptureError.alreadyStopped
    case .idle:
      break
    }

    if !configured {
      try configureOnCaptureQueue()
    }
    session.startRunning()
    guard session.isRunning else {
      throw CameraCaptureError.failedToStart
    }
    state = .running
    startFrameWatchdogOnCaptureQueue()
  }

  private func configureOnCaptureQueue() throws {
    guard
      let device = AVCaptureDevice.default(
        .builtInWideAngleCamera,
        for: .video,
        position: .back
      )
    else {
      throw CameraCaptureError.unavailable
    }

    let input = try AVCaptureDeviceInput(device: device)
    videoOutput.videoSettings = [
      kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA
    ]
    videoOutput.alwaysDiscardsLateVideoFrames = true

    session.beginConfiguration()
    defer { session.commitConfiguration() }

    if session.canSetSessionPreset(.iFrame960x540) {
      session.sessionPreset = .iFrame960x540
    } else if session.canSetSessionPreset(.hd1280x720) {
      session.sessionPreset = .hd1280x720
    } else {
      session.sessionPreset = .high
    }

    guard session.canAddInput(input) else {
      throw CameraCaptureError.cannotAddInput
    }
    guard session.canAddOutput(videoOutput) else {
      throw CameraCaptureError.cannotAddOutput
    }

    session.addInput(input)
    session.addOutput(videoOutput)
    configured = true

    guard
      let videoConnection = videoOutput.connection(with: .video),
      videoConnection.isVideoRotationAngleSupported(0)
    else {
      throw CameraCaptureError.unsupportedLandscapeRotation
    }
    // Camera Mesh intentionally has one fixed 16:9 wire orientation. Keeping
    // the sensor-native landscape angle avoids device-dependent defaults and
    // makes every emitted buffer deterministic regardless of UI rotation.
    videoConnection.videoRotationAngle = 0

    videoOutput.setSampleBufferDelegate(self, queue: captureQueue)
    installNotificationObserversOnCaptureQueue()
  }

  private func installNotificationObserversOnCaptureQueue() {
    guard notificationObservers.isEmpty else { return }

    let center = NotificationCenter.default
    notificationObservers.append(
      center.addObserver(
        forName: AVCaptureSession.runtimeErrorNotification,
        object: session,
        queue: nil
      ) { [weak self] notification in
        let reason =
          (notification.userInfo?[AVCaptureSessionErrorKey] as? NSError)?.localizedDescription
          ?? "unknown AVFoundation runtime error"
        self?.captureQueue.async { [weak self] in
          self?.stopOnCaptureQueue(
            throwing: CameraCaptureError.runtimeFailure(reason)
          )
        }
      }
    )
    notificationObservers.append(
      center.addObserver(
        forName: AVCaptureSession.wasInterruptedNotification,
        object: session,
        queue: nil
      ) { [weak self] notification in
        let reason = Self.interruptionReason(notification)
        self?.captureQueue.async { [weak self] in
          self?.stopOnCaptureQueue(
            throwing: CameraCaptureError.interrupted(reason)
          )
        }
      }
    )
    notificationObservers.append(
      center.addObserver(
        forName: AVCaptureSession.didStopRunningNotification,
        object: session,
        queue: nil
      ) { [weak self] _ in
        self?.captureQueue.async { [weak self] in
          self?.stopOnCaptureQueue(
            throwing: CameraCaptureError.stoppedUnexpectedly
          )
        }
      }
    )
  }

  private static func interruptionReason(_ notification: Notification) -> String {
    guard
      let value = notification.userInfo?[AVCaptureSessionInterruptionReasonKey] as? NSNumber
    else {
      return "the camera became unavailable"
    }
    switch value.intValue {
    case 1:
      return "the app left the foreground"
    case 2:
      return "another client is using the audio device"
    case 3:
      return "another client is using the camera"
    case 4:
      return "the camera is unavailable in the current multi-app layout"
    case 5:
      return "the device is under excessive system pressure"
    case 6:
      return "sensitive-content mitigation stopped the camera"
    default:
      return "AVFoundation reason \(value.intValue)"
    }
  }

  private func startFrameWatchdogOnCaptureQueue() {
    guard frameWatchdog == nil else { return }

    watchdogStartedUptimeNanoseconds = DispatchTime.now().uptimeNanoseconds
    lastSuccessfulJPEGUptimeNanoseconds = nil
    let watchdog = DispatchSource.makeTimerSource(queue: captureQueue)
    watchdog.schedule(
      deadline: .now() + .seconds(1),
      repeating: .seconds(1),
      leeway: .milliseconds(100)
    )
    watchdog.setEventHandler { [weak self] in
      self?.checkFrameWatchdogOnCaptureQueue()
    }
    frameWatchdog = watchdog
    watchdog.resume()
  }

  private func checkFrameWatchdogOnCaptureQueue() {
    guard state == .running else { return }
    guard
      let reference = lastSuccessfulJPEGUptimeNanoseconds
        ?? watchdogStartedUptimeNanoseconds
    else {
      return
    }

    let now = DispatchTime.now().uptimeNanoseconds
    guard now >= reference, now - reference >= Self.watchdogTimeoutNanoseconds else {
      return
    }
    stopOnCaptureQueue(throwing: CameraCaptureError.frameStalled)
  }

  private func stopOnCaptureQueue(throwing terminalError: CameraCaptureError? = nil) {
    guard state != .stopped else { return }
    state = .stopped

    frameWatchdog?.cancel()
    frameWatchdog = nil
    watchdogStartedUptimeNanoseconds = nil
    lastSuccessfulJPEGUptimeNanoseconds = nil

    let center = NotificationCenter.default
    for observer in notificationObservers {
      center.removeObserver(observer)
    }
    notificationObservers.removeAll()

    videoOutput.setSampleBufferDelegate(nil, queue: nil)
    if session.isRunning {
      session.stopRunning()
    }
    if configured {
      session.beginConfiguration()
      for output in session.outputs {
        session.removeOutput(output)
      }
      for input in session.inputs {
        session.removeInput(input)
      }
      session.commitConfiguration()
      configured = false
    }
    lastFrameUptimeNanoseconds = nil
    if let terminalError {
      frameContinuation.finish(throwing: terminalError)
    } else {
      frameContinuation.finish()
    }
  }

  private func makeJPEG(from sampleBuffer: CMSampleBuffer) -> Data? {
    guard let pixelBuffer = CMSampleBufferGetImageBuffer(sampleBuffer) else { return nil }
    let source = CIImage(cvPixelBuffer: pixelBuffer)
    let sourceExtent = source.extent.standardized
    guard sourceExtent.width > 0, sourceExtent.height > 0 else { return nil }

    let targetAspect = Self.targetWidth / Self.targetHeight
    let sourceAspect = sourceExtent.width / sourceExtent.height
    let crop: CGRect
    if sourceAspect > targetAspect {
      let width = sourceExtent.height * targetAspect
      crop = CGRect(
        x: sourceExtent.midX - width / 2,
        y: sourceExtent.minY,
        width: width,
        height: sourceExtent.height
      )
    } else {
      let height = sourceExtent.width / targetAspect
      crop = CGRect(
        x: sourceExtent.minX,
        y: sourceExtent.midY - height / 2,
        width: sourceExtent.width,
        height: height
      )
    }

    let normalized = source.cropped(to: crop).transformed(
      by: CGAffineTransform(translationX: -crop.minX, y: -crop.minY)
    )
    let resized = normalized.transformed(
      by: CGAffineTransform(
        scaleX: Self.targetWidth / crop.width,
        y: Self.targetHeight / crop.height
      )
    )
    let targetExtent = CGRect(
      x: 0,
      y: 0,
      width: Self.targetWidth,
      height: Self.targetHeight
    )
    let qualityKey = CIImageRepresentationOption(
      rawValue: kCGImageDestinationLossyCompressionQuality as String
    )
    return imageContext.jpegRepresentation(
      of: resized.cropped(to: targetExtent),
      colorSpace: colorSpace,
      options: [qualityKey: 0.7]
    )
  }
}

extension CameraCapture: AVCaptureVideoDataOutputSampleBufferDelegate {
  func captureOutput(
    _ output: AVCaptureOutput,
    didOutput sampleBuffer: CMSampleBuffer,
    from connection: AVCaptureConnection
  ) {
    guard state == .running else { return }

    let now = DispatchTime.now().uptimeNanoseconds
    if let last = lastFrameUptimeNanoseconds,
      now >= last,
      now - last < Self.frameIntervalNanoseconds
    {
      return
    }
    lastFrameUptimeNanoseconds = now

    autoreleasepool {
      guard let jpeg = makeJPEG(from: sampleBuffer) else { return }
      switch frameContinuation.yield(jpeg) {
      case .enqueued, .dropped:
        lastSuccessfulJPEGUptimeNanoseconds = DispatchTime.now().uptimeNanoseconds
      case .terminated:
        break
      @unknown default:
        break
      }
    }
  }
}
