import Foundation
import ExpoModulesCore

#if canImport(auki_sdk_swiftFFI)
final class ExpoFleetRegistry {
  private let lock = NSLock()
  private var clients: [String: AukiDomainFleet] = [:]
  private var operations: [String: AukiCancellation] = [:]
  private var cancelledBeforeStart: Set<String> = []

  func insert(_ client: AukiDomainFleet, id: String) { lock.withLock { clients[id] = client } }
  func existing(_ id: String) -> AukiDomainFleet? { lock.withLock { clients[id] } }
  func client(_ id: String) throws -> AukiDomainFleet {
    guard let client = existing(id) else {
      throw ExpoFleetFailure(kind: "closed", status: nil, code: "closed", message: "Fleet client is closed")
    }
    return client
  }
  func remove(_ id: String) { _ = lock.withLock { clients.removeValue(forKey: id) } }

  func beginOperation(_ id: String) -> AukiCancellation {
    let cancellation = AukiCancellation()
    let cancelled = lock.withLock { () -> Bool in
      operations[id] = cancellation
      return cancelledBeforeStart.remove(id) != nil
    }
    if cancelled { cancellation.cancel() }
    return cancellation
  }
  func finishOperation(_ id: String) { _ = lock.withLock { operations.removeValue(forKey: id) } }
  func cancelOperation(_ id: String) {
    let cancellation = lock.withLock { () -> AukiCancellation? in
      guard let cancellation = operations[id] else {
        if cancelledBeforeStart.count < 1_024 { cancelledBeforeStart.insert(id) }
        return nil
      }
      return cancellation
    }
    cancellation?.cancel()
  }
}

final class ExpoFleetFailure: Exception {
  private let failureCode: String
  private let failureReason: String
  init(kind: String, status: UInt16?, code: String, message: String) {
    failureCode = ["fleet", kind, status.map(String.init) ?? "", code].joined(separator: ":")
    failureReason = message
    super.init()
  }
  override var code: String { failureCode }
  override var reason: String { failureReason }
}

func withFleetErrors<T>(_ operation: () async throws -> T) async throws -> T {
  do { return try await operation() }
  catch AukiSdkError.Fleet(let kind, let status, let code, let message) {
    throw ExpoFleetFailure(kind: kind, status: status, code: code, message: message)
  }
}
#endif
