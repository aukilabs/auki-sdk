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
  case unsupportedFrameRate
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
    case .unsupportedFrameRate:
      "The back camera cannot capture the 30 fps Camera Mesh source."
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

struct CameraCaptureBatch: Sendable {
  let renditions: [CameraQuality: Data]

  subscript(_ quality: CameraQuality) -> Data? {
    renditions[quality]
  }

  var initialRenditions: CameraRenditionJPEGs? {
    guard
      let low = self[.low],
      let medium = self[.medium],
      let high = self[.high]
    else { return nil }
    return CameraRenditionJPEGs(low: low, medium: medium, high: high)
  }
}

/// A foreground, single-use source of bounded Camera Mesh JPEG frames.
///
/// Session state and `AVCaptureVideoDataOutput` callbacks share one serial
/// queue. The stream retains only two newest rendition batches, while Rust
/// retains one JPEG per quality tier, so a slow consumer cannot build an
/// unbounded queue.
final class CameraCapture: NSObject, @unchecked Sendable {
  let frames: AsyncThrowingStream<CameraCaptureBatch, any Error>

  private enum State: Equatable {
    case idle
    case running
    case stopped
  }

  private static let sourceRateHz = CameraMeshContract.profile(.high).rateHz
  private static let watchdogTimeoutNanoseconds = UInt64(3_000_000_000)
  private let frameContinuation: AsyncThrowingStream<CameraCaptureBatch, any Error>.Continuation
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
  private var captureFrameIndex: UInt64 = 0
  private var watchdogStartedUptimeNanoseconds: UInt64?
  private var lastSuccessfulJPEGUptimeNanoseconds: UInt64?
  private var frameWatchdog: DispatchSourceTimer?
  private var notificationObservers: [NSObjectProtocol] = []

  override init() {
    let pair = AsyncThrowingStream<CameraCaptureBatch, any Error>.makeStream(
      bufferingPolicy: .bufferingNewest(2)
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

    if session.canSetSessionPreset(.inputPriority) {
      session.sessionPreset = .inputPriority
    }

    guard session.canAddInput(input) else {
      throw CameraCaptureError.cannotAddInput
    }
    guard session.canAddOutput(videoOutput) else {
      throw CameraCaptureError.cannotAddOutput
    }

    session.addInput(input)
    session.addOutput(videoOutput)
    try configureDeviceFormat(device)
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

  private func configureDeviceFormat(_ device: AVCaptureDevice) throws {
    let high = CameraMeshContract.profile(.high)
    let targetRate = Double(Self.sourceRateHz)
    let candidates = device.formats.filter { format in
      let dimensions = CMVideoFormatDescriptionGetDimensions(format.formatDescription)
      return dimensions.width >= high.width
        && dimensions.height >= high.height
        && format.videoSupportedFrameRateRanges.contains { range in
          range.minFrameRate <= targetRate && range.maxFrameRate >= targetRate
        }
    }
    guard
      let format = candidates.min(by: { left, right in
        let leftDimensions = CMVideoFormatDescriptionGetDimensions(left.formatDescription)
        let rightDimensions = CMVideoFormatDescriptionGetDimensions(right.formatDescription)
        let leftPixels = Int64(leftDimensions.width) * Int64(leftDimensions.height)
        let rightPixels = Int64(rightDimensions.width) * Int64(rightDimensions.height)
        if leftPixels != rightPixels { return leftPixels < rightPixels }
        let leftMaximumRate = left.videoSupportedFrameRateRanges.map(\.maxFrameRate).max() ?? 0
        let rightMaximumRate = right.videoSupportedFrameRateRanges.map(\.maxFrameRate).max() ?? 0
        return leftMaximumRate < rightMaximumRate
      })
    else {
      throw CameraCaptureError.unsupportedFrameRate
    }

    try device.lockForConfiguration()
    defer { device.unlockForConfiguration() }
    device.activeFormat = format
    let duration = CMTime(value: 1, timescale: CMTimeScale(Self.sourceRateHz))
    device.activeVideoMinFrameDuration = duration
    device.activeVideoMaxFrameDuration = duration
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
    captureFrameIndex = 0
    if let terminalError {
      frameContinuation.finish(throwing: terminalError)
    } else {
      frameContinuation.finish()
    }
  }

  private func dueProfiles() -> [CameraStreamProfile] {
    let frameIndex = captureFrameIndex
    captureFrameIndex &+= 1
    return CameraMeshContract.profiles.filter { profile in
      let divisor = UInt64(Self.sourceRateHz / profile.rateHz)
      return frameIndex.isMultiple(of: divisor)
    }
  }

  private func makeJPEGs(
    from sampleBuffer: CMSampleBuffer,
    profiles: [CameraStreamProfile]
  ) -> [CameraQuality: Data] {
    guard let pixelBuffer = CMSampleBufferGetImageBuffer(sampleBuffer) else { return [:] }
    let source = CIImage(cvPixelBuffer: pixelBuffer)
    let sourceExtent = source.extent.standardized
    guard sourceExtent.width > 0, sourceExtent.height > 0 else { return [:] }

    let high = CameraMeshContract.profile(.high)
    let targetAspect = CGFloat(high.width) / CGFloat(high.height)
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
    let qualityKey = CIImageRepresentationOption(
      rawValue: kCGImageDestinationLossyCompressionQuality as String
    )
    var result: [CameraQuality: Data] = [:]
    for profile in profiles {
      let targetWidth = CGFloat(profile.width)
      let targetHeight = CGFloat(profile.height)
      let resized = normalized.transformed(
        by: CGAffineTransform(
          scaleX: targetWidth / crop.width,
          y: targetHeight / crop.height
        )
      )
      let targetExtent = CGRect(x: 0, y: 0, width: targetWidth, height: targetHeight)
      if let jpeg = imageContext.jpegRepresentation(
        of: resized.cropped(to: targetExtent),
        colorSpace: colorSpace,
        options: [qualityKey: 0.7]
      ) {
        result[profile.quality] = jpeg
      }
    }
    return result
  }
}

extension CameraCapture: AVCaptureVideoDataOutputSampleBufferDelegate {
  func captureOutput(
    _ output: AVCaptureOutput,
    didOutput sampleBuffer: CMSampleBuffer,
    from connection: AVCaptureConnection
  ) {
    guard state == .running else { return }

    let profiles = dueProfiles()
    guard !profiles.isEmpty else { return }

    autoreleasepool {
      let renditions = makeJPEGs(from: sampleBuffer, profiles: profiles)
      guard !renditions.isEmpty else { return }
      switch frameContinuation.yield(CameraCaptureBatch(renditions: renditions)) {
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
