import Foundation

public enum AukiJSONValue: Codable, Sendable, Equatable {
    case null
    case bool(Bool)
    case integer(Int64)
    case unsignedInteger(UInt64)
    case number(Double)
    case string(String)
    case array([AukiJSONValue])
    case object([String: AukiJSONValue])

    public init(from decoder: Decoder) throws {
        let value = try decoder.singleValueContainer()
        if value.decodeNil() { self = .null }
        else if let decoded = try? value.decode(Bool.self) { self = .bool(decoded) }
        else if let decoded = try? value.decode(Int64.self) { self = .integer(decoded) }
        else if let decoded = try? value.decode(UInt64.self) { self = .unsignedInteger(decoded) }
        else if let decoded = try? value.decode(Double.self) { self = .number(decoded) }
        else if let decoded = try? value.decode(String.self) { self = .string(decoded) }
        else if let decoded = try? value.decode([AukiJSONValue].self) { self = .array(decoded) }
        else { self = .object(try value.decode([String: AukiJSONValue].self)) }
    }

    public func encode(to encoder: Encoder) throws {
        var value = encoder.singleValueContainer()
        switch self {
        case .null: try value.encodeNil()
        case .bool(let decoded): try value.encode(decoded)
        case .integer(let decoded): try value.encode(decoded)
        case .unsignedInteger(let decoded): try value.encode(decoded)
        case .number(let decoded): try value.encode(decoded)
        case .string(let decoded): try value.encode(decoded)
        case .array(let decoded): try value.encode(decoded)
        case .object(let decoded): try value.encode(decoded)
        }
    }
}

public enum AukiJobMode: String, Codable, Sendable { case `public`, dedicated }
public enum AukiJobStatus: String, Codable, Sendable { case pending, running, completed, failed, canceled }
public enum AukiJobTaskStatus: String, Codable, Sendable { case queued, leased, running, completed, failed, canceled }

public struct AukiJobTaskSpec: Codable, Sendable {
    public var label: String
    public var stage: String
    public var capability: String
    public var mode: AukiJobMode
    public var capabilityFilters: [String: String]
    public var priority: Int32
    public var inputsCids: [String]
    public var outputsPrefix: String?
    public var meta: [String: AukiJSONValue]
    public var maxAttempts: UInt32

    public init(label: String, stage: String, capability: String, mode: AukiJobMode = .public,
                capabilityFilters: [String: String] = [:], priority: Int32 = 0,
                inputsCids: [String] = [], outputsPrefix: String? = nil,
                meta: [String: AukiJSONValue] = [:], maxAttempts: UInt32 = 3) {
        self.label = label; self.stage = stage; self.capability = capability; self.mode = mode
        self.capabilityFilters = capabilityFilters; self.priority = priority
        self.inputsCids = inputsCids; self.outputsPrefix = outputsPrefix; self.meta = meta
        self.maxAttempts = maxAttempts
    }
}

public struct AukiJobEdge: Codable, Sendable {
    public var from: String
    public var to: String
    public init(from: String, to: String) { self.from = from; self.to = to }
}

public struct AukiJobSpec: Codable, Sendable {
    public var label: String
    public var priority: UInt32
    public var meta: [String: AukiJSONValue]
    public var tasks: [AukiJobTaskSpec]
    public var edges: [AukiJobEdge]

    public init(label: String, priority: UInt32 = 0, meta: [String: AukiJSONValue] = [:],
                tasks: [AukiJobTaskSpec], edges: [AukiJobEdge] = []) {
        self.label = label; self.priority = priority; self.meta = meta
        self.tasks = tasks; self.edges = edges
    }
}

public struct AukiJobListQuery: Codable, Sendable {
    public var limit: UInt32
    public var cursor: String?
    public var status: AukiJobStatus?
    public var capabilities: [String]
    public var matchAllCapabilities: Bool

    public init(limit: UInt32 = 50, cursor: String? = nil, status: AukiJobStatus? = nil,
                capabilities: [String] = [], matchAllCapabilities: Bool = false) {
        self.limit = limit; self.cursor = cursor; self.status = status
        self.capabilities = capabilities; self.matchAllCapabilities = matchAllCapabilities
    }
}

public struct AukiJobEstimateTask: Codable, Sendable {
    public let label: String; public let stage: String; public let capability: String
    public let mode: AukiJobMode; public let billingUnits: String; public let estimatedCreditCost: String
}
public struct AukiJobEstimate: Codable, Sendable { public let total: String; public let tasks: [AukiJobEstimateTask] }

public struct AukiJobRecord: Codable, Sendable {
    public let id: String; public let label: String; public let domainId: String
    public let status: AukiJobStatus; public let priority: UInt32
    public let createdAt: String; public let updatedAt: String; public let organizationId: String?
    public let meta: AukiJSONValue; public let creditLockId: String?; public let creditLockAmount: String?
    public let creditLockedAt: String?; public let creditReleasedAt: String?
}

public struct AukiJobTaskSummary: Codable, Sendable {
    public let queued: UInt32; public let leased: UInt32; public let running: UInt32
    public let completed: UInt32; public let failed: UInt32; public let canceled: UInt32
}
public struct AukiJobListItem: Codable, Sendable { public let job: AukiJobRecord; public let tasksSummary: AukiJobTaskSummary }
public struct AukiJobPage: Codable, Sendable { public let items: [AukiJobListItem]; public let nextCursor: String? }

public struct AukiJobTask: Codable, Sendable {
    public let id: String; public let jobId: String; public let label: String; public let stage: String
    public let capability: String; public let capabilityFilters: [String: String]; public let status: AukiJobTaskStatus
    public let depsRemaining: UInt32; public let priority: Int32; public let inputsCids: [String]
    public let outputsPrefix: String?; public let organizationId: String?; public let attempts: UInt32
    public let maxAttempts: UInt32; public let leaseExpiresAt: String?; public let reservedBy: String?
    public let meta: AukiJSONValue; public let cancelRequestedAt: String?; public let lastHeartbeatAt: String?
    public let createdAt: String; public let updatedAt: String; public let mode: AukiJobMode
    public let billingUnits: String; public let estimatedCreditCost: String?; public let debitedAmount: String?
    public let debitedAt: String?
}

public struct AukiJobReceipt: Codable, Sendable {
    public let id: String; public let jobId: String; public let taskId: String; public let nodeId: String?
    public let outputs: [String]; public let meta: AukiJSONValue; public let createdAt: String
}
public struct AukiJobDetails: Codable, Sendable {
    public let job: AukiJobRecord; public let tasksSummary: AukiJobTaskSummary
    public let tasks: [AukiJobTask]; public let receipts: [AukiJobReceipt]
}
public struct AukiJobCancellation: Codable, Sendable {
    public let id: String; public let status: AukiJobStatus; public let updatedAt: String
}

private enum AukiJobsJSON {
    static let encoder: JSONEncoder = {
        let value = JSONEncoder(); value.keyEncodingStrategy = .convertToSnakeCase; return value
    }()
    static let decoder: JSONDecoder = {
        let value = JSONDecoder(); value.keyDecodingStrategy = .convertFromSnakeCase; return value
    }()
    static func encode<T: Encodable>(_ value: T) throws -> String {
        String(decoding: try encoder.encode(value), as: UTF8.self)
    }
    static func decode<T: Decodable>(_ type: T.Type, _ json: String) throws -> T {
        try decoder.decode(type, from: Data(json.utf8))
    }
}

public extension AukiDomainJobs {
    func estimate(_ spec: AukiJobSpec, cancellation: AukiCancellation? = nil) async throws -> AukiJobEstimate {
        try AukiJobsJSON.decode(AukiJobEstimate.self,
            try await estimateJson(specJson: AukiJobsJSON.encode(spec), cancellation: cancellation))
    }
    func submit(_ spec: AukiJobSpec, cancellation: AukiCancellation? = nil) async throws -> String {
        try await submitJson(specJson: AukiJobsJSON.encode(spec), cancellation: cancellation)
    }
    func list(_ query: AukiJobListQuery = .init(), cancellation: AukiCancellation? = nil) async throws -> AukiJobPage {
        try AukiJobsJSON.decode(AukiJobPage.self,
            try await listJson(queryJson: AukiJobsJSON.encode(query), cancellation: cancellation))
    }
    func get(_ jobId: String, cancellation: AukiCancellation? = nil) async throws -> AukiJobDetails {
        try AukiJobsJSON.decode(AukiJobDetails.self,
            try await getJson(jobId: jobId, cancellation: cancellation))
    }
    func cancel(_ jobId: String, cancellation: AukiCancellation? = nil) async throws -> AukiJobCancellation {
        try AukiJobsJSON.decode(AukiJobCancellation.self,
            try await cancelJson(jobId: jobId, cancellation: cancellation))
    }
}
