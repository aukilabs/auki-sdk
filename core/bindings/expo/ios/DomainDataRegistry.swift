import Foundation
import ExpoModulesCore

#if canImport(auki_sdk_uniffiFFI)
private struct DownloadEntry {
  let transfer: AukiDataDownload
  let operationId: String
}

private struct UploadEntry {
  let transfer: AukiDataUpload
  let operationId: String
}

/// Expo async functions may run concurrently. Registry locks only protect
/// handle lookup; no lock is held across an SDK await or transfer cleanup.
final class ExpoDomainDataRegistry {
  private let lock = NSLock()
  private var clients: [String: AukiDomainData] = [:]
  private var operations: [String: AukiCancellation] = [:]
  private var cancelledBeforeStart: Set<String> = []
  private var downloads: [String: DownloadEntry] = [:]
  private var uploads: [String: UploadEntry] = [:]

  func insertClient(_ client: AukiDomainData, id: String) {
    lock.withLock { clients[id] = client }
  }

  func client(_ id: String) throws -> AukiDomainData {
    guard let client = lock.withLock({ clients[id] }) else {
      throw ExpoDataFailure(kind: "closed", status: nil, message: "Domain data client is closed")
    }
    return client
  }

  func removeClient(_ id: String) -> AukiDomainData? {
    lock.withLock { clients.removeValue(forKey: id) }
  }

  func beginOperation(_ id: String) -> AukiCancellation {
    let cancellation = AukiCancellation()
    let cancelled = lock.withLock { () -> Bool in
      operations[id] = cancellation
      return cancelledBeforeStart.remove(id) != nil
    }
    if cancelled { cancellation.cancel() }
    return cancellation
  }

  func finishOperation(_ id: String) {
    _ = lock.withLock { operations.removeValue(forKey: id) }
  }

  func cancelOperation(_ id: String) {
    let cancellation = lock.withLock { () -> AukiCancellation? in
      guard let cancellation = operations[id] else {
        // AbortSignal can fire while an Expo call is entering native code.
        // Keep this bounded against arbitrary raw bridge callers.
        if cancelledBeforeStart.count < 1_024 { cancelledBeforeStart.insert(id) }
        return nil
      }
      return cancellation
    }
    cancellation?.cancel()
  }

  func insertDownload(
    _ transfer: AukiDataDownload,
    id: String,
    operationId: String
  ) {
    lock.withLock { downloads[id] = DownloadEntry(transfer: transfer, operationId: operationId) }
  }

  func download(_ id: String) throws -> AukiDataDownload {
    guard let entry = lock.withLock({ downloads[id] }) else {
      throw ExpoDataFailure(kind: "closed", status: nil, message: "Domain data download is closed")
    }
    return entry.transfer
  }

  func removeDownload(_ id: String) -> AukiDataDownload? {
    let entry = lock.withLock { downloads.removeValue(forKey: id) }
    if let entry { finishOperation(entry.operationId) }
    return entry?.transfer
  }

  func insertUpload(
    _ transfer: AukiDataUpload,
    id: String,
    operationId: String
  ) {
    lock.withLock { uploads[id] = UploadEntry(transfer: transfer, operationId: operationId) }
  }

  func upload(_ id: String) throws -> AukiDataUpload {
    guard let entry = lock.withLock({ uploads[id] }) else {
      throw ExpoDataFailure(kind: "closed", status: nil, message: "Domain data upload is closed")
    }
    return entry.transfer
  }

  func removeUpload(_ id: String) -> AukiDataUpload? {
    let entry = lock.withLock { uploads.removeValue(forKey: id) }
    if let entry { finishOperation(entry.operationId) }
    return entry?.transfer
  }
}

final class ExpoDataFailure: Exception {
  private let failureCode: String
  private let failureReason: String

  init(kind: String, status: UInt16?, authKind: AukiAuthFailureKind? = nil, message: String) {
    failureCode = [
      "domain_data",
      kind,
      status.map(String.init) ?? "",
      authKind.map(expoAuthFailureCode) ?? "",
    ].joined(separator: ":")
    failureReason = message
    super.init()
  }

  override var code: String { failureCode }
  override var reason: String { failureReason }
}

func withDataErrors<T>(_ operation: () async throws -> T) async throws -> T {
  do { return try await operation() }
  catch AukiSdkError.DomainData(let kind, let status, let authKind, let message) {
    throw ExpoDataFailure(
      kind: expoDataFailureKind(kind),
      status: status,
      authKind: authKind,
      message: message
    )
  }
}

private func expoDataFailureKind(_ kind: AukiDataFailureKind) -> String {
  switch kind {
  case .authentication: return "auth"
  case .http: return "http"
  case .invalidInput: return "input"
  case .invalidResponse: return "response"
  case .limit: return "limit"
  case .cancelled: return "cancelled"
  case .closed: return "closed"
  case .timeout: return "timeout"
  case .transport: return "transport"
  case .callback: return "callback"
  case .cleanup: return "cleanup"
  }
}

private func expoAuthFailureCode(_ kind: AukiAuthFailureKind) -> String {
  switch kind {
  case .authenticationRequired: return "authentication_required"
  case .configuration: return "configuration"
  case .authorizationDenied: return "authorization_denied"
  case .persistence: return "persistence"
  case .transient: return "transient"
  case .cancelled: return "cancelled"
  case .closed: return "closed"
  }
}
#endif
