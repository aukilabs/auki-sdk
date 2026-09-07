// Actual generated UniFFI host callback proof. Synthetic loopback fixture only.
import Foundation

struct HostTestFailure: Error { let message: String }
func check(_ condition: Bool, _ message: String) throws {
    if !condition { throw HostTestFailure(message: message) }
}
func authFailure(_ kind: AukiAuthFailureKind, _ operation: () async throws -> Void) async throws {
    do { try await operation(); throw HostTestFailure(message: "expected auth failure \(kind)") }
    catch AukiSdkError.Authentication(let actual) { try check(actual == kind, "incorrect auth classification") }
}
let fixtureBase = "http://127.0.0.1:18111"
func fixture(_ path: String, _ body: [String: Any]? = nil) async throws -> [String: Int] {
    var request = URLRequest(url: URL(string: fixtureBase + path)!)
    if let body { request.httpMethod = "POST"; request.httpBody = try JSONSerialization.data(withJSONObject: body) }
    let (data, response) = try await URLSession.shared.data(for: request)
    try check((response as? HTTPURLResponse)?.statusCode == 200, "fixture HTTP failed")
    return (try JSONSerialization.jsonObject(with: data) as? [String: Int]) ?? [:]
}
func credentials() throws -> AukiZitadelCredentials {
    try AukiZitadelCredentials(accessToken: "access-0", refreshToken: "refresh-0", clientId: "bindings-client",
        issuer: fixtureBase, accessTokenExpiresAt: "2020-01-01T07:00:00.123456789+07:00")
}
func imported(_ store: Store, _ restored: AukiZitadelCredentials? = nil) throws -> AukiSession {
    try AukiSession.importZitadelWithEnvironment(apiBaseUrl: fixtureBase, ddsBaseUrl: fixtureBase,
        dmsBaseUrl: fixtureBase, credentials: try restored ?? credentials(), store: store)
}

actor Gate {
    var opened = false
    var waiters: [CheckedContinuation<Void, Never>] = []
    func wait() async {
        if opened { return }
        await withCheckedContinuation { waiters.append($0) }
    }
    func open() { opened = true; let pending = waiters; waiters = []; for waiter in pending { waiter.resume() } }
}
actor Store: AukiZitadelSessionStore {
    let entered = Gate()
    let release = Gate()
    var blocked: Bool
    var fail: Bool
    var unexpected: Bool
    var calls = 0
    var durable: AukiZitadelCredentials?
    var generations: [String] = []
    init(blocked: Bool = false, fail: Bool = false, unexpected: Bool = false) {
        self.blocked = blocked; self.fail = fail; self.unexpected = unexpected
    }
    func save(credentials: AukiZitadelCredentials) async throws {
        calls += 1; generations.append(credentials.exposeRefreshToken())
        await entered.open()
        if blocked { await release.wait() }
        if fail {
            if unexpected { throw HostTestFailure(message: "DO_NOT_LEAK_HOST_SECRET") }
            throw AukiPersistenceError.Failed
        }
        durable = credentials // in-memory atomic test store, not a production persistence example
    }
    func allowSave() { fail = false }
    func clear() { durable = nil }
}

@main struct SwiftHost {
    static func main() async throws {
        let watchdog = Task.detached {
            do { try await Task.sleep(for: .seconds(45)) } catch { return }
            print("FAIL Swift host test timeout"); exit(1)
        }
        defer { watchdog.cancel() }
        let initial = try credentials()
        try check(initial.accessTokenExpiresAt() == "2020-01-01T00:00:00.123456789Z", "nanoseconds/timezone lost in Swift FFI")
        let description = String(reflecting: initial)
        try check(!description.contains("access-0") && !description.contains("refresh-0"), "credential diagnostics exposed secrets")
        let unknown = try AukiZitadelCredentials(accessToken: "a", refreshToken: "r", clientId: "c", issuer: fixtureBase, accessTokenExpiresAt: nil)
        try check(unknown.accessTokenExpiresAt() == nil, "unknown expiry changed")
        // Exercise generated FFI RustBuffer conversion, comparing bytes, not
        // Swift's canonical-equivalence String equality.
        let exactSubject = " User|Case-敏感-e\u{301} "
        let peer = AukiMessageAuthenticatedPeer(peerId: "test-peer", subject: exactSubject, peerType: "user",
            domainIds: [], scopes: [], application: nil, verifiedUntil: "2026-09-07T00:00:00.123456789Z")
        let roundtrip = try FfiConverterTypeAukiMessageAuthenticatedPeer_lift(FfiConverterTypeAukiMessageAuthenticatedPeer_lower(peer))
        try check(Array(roundtrip.subject.utf8) == Array(exactSubject.utf8), "subject bytes changed at FFI boundary")
        try check(roundtrip.verifiedUntil == peer.verifiedUntil, "timestamp changed at FFI boundary")
        print("PASS Swift exact subjects, timestamps, redacted credentials")

        _ = try await fixture("/__reset", [:])
        let store = Store(blocked: true)
        let session = try imported(store)
        let initialStats = try await fixture("/__stats")
        try check(initialStats["refresh"] == 0 && initialStats["exchange"] == 0, "import performed network I/O")
        let first = Task { try await session.accessibleDomains() }
        let second = Task { try await session.accessibleDomains() }
        await store.entered.wait()
        let pendingStats = try await fixture("/__stats")
        try check(pendingStats["exchange"] == 0, "exchange before host ACK")
        await store.release.open()
        let domains = try await first.value
        try check(domains.count == 1 && domains[0].name == "Readable Domain", "domain selection wrong")
        try check(try await second.value.count == 1, "concurrent domain lookup failed")
        try check(await store.calls == 1, "duplicated host save")
        try await authFailure(.authorizationDenied) {
            _ = try await session.startPeer(domainId: "00000000-0000-0000-0000-000000000099", identity: AukiPeerIdentity.generate())
        }
        let saved = await store.durable!
        await session.close()
        let restoredStore = Store()
        let restored = try imported(restoredStore, saved)
        try check(try await restored.accessibleDomains().count == 1, "restart failed")
        let restartedStats = try await fixture("/__stats")
        try check(restartedStats["refresh"] == 1, "restart replayed rotation")
        await restored.close()
        print("PASS Swift single-flight, ACK ordering, selected Domain denial, restart")

        _ = try await fixture("/__reset", [:])
        let failingStore = Store(fail: true)
        let recoverable = try imported(failingStore)
        try await authFailure(.persistence) { _ = try await recoverable.accessibleDomains() }
        let failedStats = try await fixture("/__stats")
        try check(failedStats["exchange"] == 0, "failed save permitted network")
        await failingStore.allowSave()
        try check(try await recoverable.accessibleDomains().count == 1, "save retry failed")
        try check(await failingStore.generations == ["refresh-1", "refresh-1"], "save retry rotated or lost credentials")
        await recoverable.close()
        print("PASS Swift thrown host save retains replacement and retries ACK")

        _ = try await fixture("/__reset", [:])
        let unexpectedStore = Store(fail: true, unexpected: true)
        let unexpected = try imported(unexpectedStore)
        try await authFailure(.persistence) { _ = try await unexpected.accessibleDomains() }
        await unexpectedStore.allowSave()
        try check(try await unexpected.accessibleDomains().count == 1, "unexpected host error poisoned recoverable session")
        try check(await unexpectedStore.generations == ["refresh-1", "refresh-1"], "unexpected host error replayed grant")
        await unexpected.close()
        print("PASS Swift unexpected host errors are redacted persistence failures, not panics")

        _ = try await fixture("/__reset", ["exchangeFailure": 503])
        let retryStore = Store()
        let retry = try imported(retryStore)
        try await authFailure(.transient) { _ = try await retry.accessibleDomains() }
        _ = try await fixture("/__configure", ["exchangeFailure": NSNull()])
        try check(try await retry.accessibleDomains().count == 1, "startup handle could not recover")
        try check(await retryStore.calls == 1, "startup retry duplicated refresh")
        await retry.close()
        print("PASS Swift retained handle after startup exchange failure")

        _ = try await fixture("/__reset", [:])
        let closingStore = Store(blocked: true)
        let closingSession = try imported(closingStore)
        let lookup = Task { try await authFailure(.closed) { _ = try await closingSession.accessibleDomains() } }
        await closingStore.entered.wait()
        let closed = Gate()
        let closing = Task { await closingSession.close(); await closed.open() }
        try await Task.sleep(for: .milliseconds(100))
        try check(await !closed.opened, "close finished during a pending host save")
        // Cancellation of the observing Swift Task must not abandon the save.
        closing.cancel()
        await closingStore.release.open()
        await closing.value
        await closingSession.close() // repeat the drain if the first wait was cancelled
        try await lookup.value
        await closingStore.clear()
        try await authFailure(.closed) { _ = try await closingSession.accessibleDomains() }
        try await Task.sleep(for: .milliseconds(50))
        try check(await closingStore.durable == nil, "late write resurrected logged-out session")
        let closeStats = try await fixture("/__stats")
        try check(closeStats["exchange"] == 0, "close allowed downstream work")
        print("PASS Swift close/cancellation drains pending write before clear")

        for (oauth, kind) in [("invalid_grant", AukiAuthFailureKind.authenticationRequired), ("invalid_client", .configuration)] {
            _ = try await fixture("/__reset", ["tokenError": oauth])
            let terminal = try imported(Store())
            try await authFailure(kind) { _ = try await terminal.accessibleDomains() }
            try await authFailure(kind) { _ = try await terminal.accessibleDomains() }
            let stats = try await fixture("/__stats")
            try check(stats["refresh"] == 1, "terminal failure retried grant")
            await terminal.close()
        }
        print("PASS Swift typed terminal errors latch without retry")
        for (issuer, expiry) in [("DO_NOT_LEAK_SECRET", "2026-09-07T00:00:00Z"), (fixtureBase, "DO_NOT_LEAK_SECRET")] {
            do {
                _ = try AukiZitadelCredentials(accessToken: "a", refreshToken: "r", clientId: "client", issuer: issuer, accessTokenExpiresAt: expiry)
                throw HostTestFailure(message: "invalid configuration accepted")
            } catch AukiSdkError.Authentication(let kind) { try check(kind == .configuration, "validation error lost kind") }
        }
        print("PASS Swift redacted configuration errors")
        print("PASS 8 Swift binding cases")
    }
}
