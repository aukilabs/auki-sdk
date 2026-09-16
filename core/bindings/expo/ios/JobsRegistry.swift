import Foundation
import ExpoModulesCore

#if canImport(auki_sdk_swiftFFI)
final class ExpoJobsRegistry {
  private let lock = NSLock()
  private var clients: [String: AukiDomainJobs] = [:]
  private var operations: [String: AukiCancellation] = [:]
  private var cancelledBeforeStart: Set<String> = []

  func insert(_ client: AukiDomainJobs, id: String) { lock.withLock { clients[id] = client } }

  func client(_ id: String) throws -> AukiDomainJobs {
    guard let client = lock.withLock({ clients[id] }) else {
      throw ExpoJobsFailure(kind: "closed", status: nil, code: nil, maximum: nil,
        source: nil, message: "DMS jobs client is closed")
    }
    return client
  }

  func remove(_ id: String) -> AukiDomainJobs? { lock.withLock { clients.removeValue(forKey: id) } }

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

final class ExpoJobsFailure: Exception {
  private let failureCode: String
  private let failureReason: String

  init(kind: String, status: UInt16?, code: String?, maximum: UInt64?, source: String?, message: String) {
    let authCode = kind == "auth" ? code ?? "" : ""
    failureCode = ["jobs", kind, status.map(String.init) ?? "", authCode, source ?? ""]
      .joined(separator: ":")
    var reason = message
    if let maximum { reason += " (maximum: \(maximum) bytes)" }
    failureReason = reason
    super.init()
  }

  override var code: String { failureCode }
  override var reason: String { failureReason }
}

func withJobsErrors<T>(_ operation: () async throws -> T) async throws -> T {
  do { return try await operation() }
  catch AukiSdkError.Jobs(let kind, let status, let code, let maximum, let sourceCode, let message) {
    throw ExpoJobsFailure(kind: expoJobsFailureKind(kind), status: status, code: code,
      maximum: maximum, source: sourceCode, message: message)
  }
}

private func expoJobsFailureKind(_ kind: AukiJobsFailureKind) -> String {
  switch kind {
  case .authentication: return "auth"
  case .invalidInput: return "invalid_input"
  case .invalidResponse: return "invalid_response"
  case .httpStatus: return "http_status"
  case .transport: return "transport"
  case .timedOut: return "timed_out"
  case .cancelled: return "cancelled"
  case .closed: return "closed"
  case .tooLarge: return "too_large"
  case .submissionUncertain: return "submission_uncertain"
  }
}
#endif
