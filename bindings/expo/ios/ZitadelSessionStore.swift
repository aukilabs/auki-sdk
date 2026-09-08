import Foundation
import ExpoModulesCore

#if canImport(auki_sdk_swiftFFI)
// No secret-bearing record is included in event payloads or exception text.
private struct ZitadelPayload: Codable {
  let accessToken: String
  let refreshToken: String
  let clientId: String
  let issuer: String
  let accessTokenExpiresAt: String?

  init(_ credentials: AukiZitadelCredentials) {
    accessToken = credentials.exposeAccessToken()
    refreshToken = credentials.exposeRefreshToken()
    clientId = credentials.clientId()
    issuer = credentials.issuer()
    accessTokenExpiresAt = credentials.accessTokenExpiresAt()
  }

  func credentials() throws -> AukiZitadelCredentials {
    try AukiZitadelCredentials(accessToken: accessToken, refreshToken: refreshToken,
      clientId: clientId, issuer: issuer, accessTokenExpiresAt: accessTokenExpiresAt)
  }
}

private struct ServiceEnvironment: Decodable {
  let apiBaseUrl: String
  let ddsBaseUrl: String
  let dmsBaseUrl: String
}

final class ExpoAuthFailure: Exception {
  let authCode: String
  init(_ kind: AukiAuthFailureKind) {
    switch kind {
    case .authenticationRequired: authCode = "authentication_required"
    case .configuration: authCode = "configuration"
    case .authorizationDenied: authCode = "authorization_denied"
    case .persistence: authCode = "persistence"
    case .transient: authCode = "transient"
    case .cancelled: authCode = "cancelled"
    case .closed: authCode = "closed"
    }
    super.init()
  }
  override var code: String { authCode }
  override var reason: String { "Auki authentication: \(authCode)" }
}

func withAuthErrors<T>(_ operation: () async throws -> T) async throws -> T {
  do { return try await operation() }
  catch AukiSdkError.Authentication(let kind) { throw ExpoAuthFailure(kind) }
}

/// At most one request belongs to each core session's serialized store. Close
/// never cancels this continuation: JS must settle every write and acknowledge.
actor ExpoZitadelStore: AukiZitadelSessionStore {
  private let notify: (String) -> Void
  private var pending: (id: String, credentials: AukiZitadelCredentials, ack: CheckedContinuation<Bool, Never>)?

  init(notify: @escaping (String) -> Void) { self.notify = notify }

  func save(credentials: AukiZitadelCredentials) async throws {
    guard pending == nil else { throw AukiPersistenceError.Failed }
    let requestId = UUID().uuidString
    let success = await withCheckedContinuation { continuation in
      pending = (requestId, credentials, continuation)
      notify(requestId)
    }
    if !success { throw AukiPersistenceError.Failed }
  }

  func credentialsJson(requestId: String) throws -> String {
    guard let current = pending, current.id == requestId else { throw ExpoAuthFailure(.persistence) }
    do { return String(decoding: try JSONEncoder().encode(ZitadelPayload(current.credentials)), as: UTF8.self) }
    catch { throw ExpoAuthFailure(.persistence) }
  }

  func acknowledge(requestId: String, success: Bool) -> Bool {
    guard let current = pending, current.id == requestId else { return false }
    pending = nil
    current.ack.resume(returning: success)
    return true
  }
}

/// Expo async methods may run concurrently. Lock only registry access; never
/// hold a lock while awaiting auth, host storage, or close.
final class ExpoSessionRegistry {
  private let lock = NSLock()
  private var sessions: [String: AukiSession] = [:]
  private var stores: [String: ExpoZitadelStore] = [:]

  func insert(id: String, session: AukiSession, store: ExpoZitadelStore? = nil) {
    lock.withLock { sessions[id] = session; stores[id] = store }
  }
  func session(_ id: String) throws -> AukiSession {
    guard let session = lock.withLock({ sessions[id] }) else { throw ExpoAuthFailure(.closed) }
    return session
  }
  func store(_ id: String) throws -> ExpoZitadelStore {
    guard let store = lock.withLock({ stores[id] }) else { throw ExpoAuthFailure(.closed) }
    return store
  }
  func close(_ id: String) async {
    guard let session = lock.withLock({ sessions[id] }) else { return }
    await session.close()
    lock.withLock { sessions.removeValue(forKey: id); stores.removeValue(forKey: id) }
  }
}

func importZitadel(credentialsJson: String, environmentJson: String?, store: ExpoZitadelStore) throws -> AukiSession {
  do {
    guard credentialsJson.utf8.count <= 2_097_152 else { throw ExpoAuthFailure(.configuration) }
    let payload = try JSONDecoder().decode(ZitadelPayload.self, from: Data(credentialsJson.utf8))
    let credentials = try payload.credentials()
    if let environmentJson {
      let environment = try JSONDecoder().decode(ServiceEnvironment.self, from: Data(environmentJson.utf8))
      return try AukiSession.importZitadelWithEnvironment(apiBaseUrl: environment.apiBaseUrl,
        ddsBaseUrl: environment.ddsBaseUrl, dmsBaseUrl: environment.dmsBaseUrl,
        credentials: credentials, store: store)
    }
    return try AukiSession.importZitadelDev(credentials: credentials, store: store)
  } catch {
    // JSON decoding/provider values must never become platform error messages.
    throw ExpoAuthFailure(.configuration)
  }
}
#endif
