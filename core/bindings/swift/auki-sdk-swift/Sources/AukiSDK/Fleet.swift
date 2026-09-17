import Foundation

public enum AukiFleetPresence: String, Codable, Sendable { case online, offline, unknown }
public enum AukiFleetWorkState: String, Codable, Sendable { case idle, busy, unknown }
public enum AukiFleetMachineKind: String, Codable, Sendable { case robot, compute }
public enum AukiFleetAssociation: String, Codable, Sendable {
    case assigned, activeTask = "active_task", candidate
}
public enum AukiFleetView: String, Codable, Sendable { case domain, computePool = "compute_pool" }
public enum AukiFleetSource: String, Codable, Sendable { case robots, nodes, jobs, busy }
public enum AukiFleetSourceState: String, Codable, Sendable {
    case complete, partial, denied, unsupported, unavailable
}

public struct AukiFleetQuery: Codable, Sendable {
    public var capabilities: [String]
    public var matchAllCapabilities: Bool
    public init(capabilities: [String] = [], matchAllCapabilities: Bool = false) {
        self.capabilities = capabilities
        self.matchAllCapabilities = matchAllCapabilities
    }
}
public struct AukiComputePoolQuery: Codable, Sendable {
    public var mode: AukiJobMode
    public var capabilities: [String]
    public var matchAllCapabilities: Bool
    public init(mode: AukiJobMode, capabilities: [String] = [], matchAllCapabilities: Bool = false) {
        self.mode = mode; self.capabilities = capabilities
        self.matchAllCapabilities = matchAllCapabilities
    }
}
public struct AukiFleetActivity: Codable, Sendable {
    public let workerId: String
    public let jobId: String
    public let taskId: String
    public let taskStatus: AukiJobTaskStatus
    public let jobStatus: AukiJobStatus
    public let mode: AukiJobMode
    public let capability: String
    public let leaseExpiresAt: String?
    public let lastHeartbeatAt: String?
    public let updatedAt: String
    public let observedAt: String
}
public struct AukiFleetMachine: Codable, Sendable {
    public let kind: AukiFleetMachineKind
    public let id: String
    public let organizationId: String
    public let name: String
    public let capabilities: [String]
    public let mode: String
    public let association: AukiFleetAssociation
    public let presence: AukiFleetPresence
    public let providerStatus: String
    public let presenceObservedAt: String
    public let lastSeenAt: String?
    public let activeLeaseExpiresAt: String?
    public let workState: AukiFleetWorkState
    public let workObservedAt: String?
    public let activity: [AukiFleetActivity]
}
public struct AukiFleetSourceReport: Codable, Sendable {
    public let source: AukiFleetSource
    public let state: AukiFleetSourceState
    public let observedAt: String
    public let codes: [String]
    public let httpStatus: UInt16?
}
public struct AukiFleetSnapshot: Codable, Sendable {
    public let domainId: String
    public let view: AukiFleetView
    public let observedAt: String
    public let machines: [AukiFleetMachine]
    public let unresolvedActivity: [AukiFleetActivity]
    public let sources: [AukiFleetSourceReport]
    public let complete: Bool
}

private enum AukiFleetJSON {
    static func encode<T: Encodable>(_ value: T) throws -> String {
        let encoder = JSONEncoder()
        encoder.keyEncodingStrategy = .convertToSnakeCase
        return String(decoding: try encoder.encode(value), as: UTF8.self)
    }
    static func decode(_ json: String) throws -> AukiFleetSnapshot {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return try decoder.decode(AukiFleetSnapshot.self, from: Data(json.utf8))
    }
}

public extension AukiDomainFleet {
    func list(_ query: AukiFleetQuery = .init(), cancellation: AukiCancellation? = nil) async throws -> AukiFleetSnapshot {
        try AukiFleetJSON.decode(try await listJson(queryJson: AukiFleetJSON.encode(query), cancellation: cancellation))
    }
    func computePool(_ query: AukiComputePoolQuery, cancellation: AukiCancellation? = nil) async throws -> AukiFleetSnapshot {
        try AukiFleetJSON.decode(try await computePoolJson(queryJson: AukiFleetJSON.encode(query), cancellation: cancellation))
    }
}
