import Foundation

private enum HostFailure: Error { case assertion(String) }

private final class MockJobs: AukiDomainJobs, @unchecked Sendable {
    var specification = ""
    var idempotencyKey = ""
    var cancellation: AukiCancellation?
    var query = ""
    var closeCount = 0

    init() { super.init(noHandle: .init()) }
    required init(unsafeFromHandle handle: UInt64) { super.init(unsafeFromHandle: handle) }

    override func estimateJson(specJson: String, cancellation: AukiCancellation?) async throws -> String {
        specification = specJson
        return #"{"total":"12.500","tasks":[{"label":"vendor","stage":"analyze","capability":"vendor.example/private-model/v7","mode":"dedicated","billing_units":"seconds","estimated_credit_cost":"12.500"}]}"#
    }

    override func submitJson(specJson: String, cancellation: AukiCancellation?) async throws -> String {
        specification = specJson
        return "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"
    }

    override func submitWithKeyJson(specJson: String, idempotencyKey: String,
                                    cancellation: AukiCancellation?) async throws -> String {
        specification = specJson
        self.idempotencyKey = idempotencyKey
        self.cancellation = cancellation
        return "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"
    }

    override func listJson(queryJson: String, cancellation: AukiCancellation?) async throws -> String {
        query = queryJson
        return #"{"items":[],"next_cursor":"opaque"}"#
    }

    override func close() async throws { closeCount += 1 }
}

@main
private struct JobsHost {
    static func require(_ condition: @autoclosure () -> Bool, _ message: String) throws {
        guard condition() else { throw HostFailure.assertion(message) }
    }

    static func object(_ json: String) throws -> [String: Any] {
        try JSONSerialization.jsonObject(with: Data(json.utf8)) as! [String: Any]
    }

    static func main() async throws {
        let jobs = MockJobs()
        let spec = AukiJobSpec(label: "custom", tasks: [AukiJobTaskSpec(
            label: "vendor", stage: "analyze",
            capability: "vendor.example/private-model/v7", mode: .dedicated,
            capabilityFilters: ["model": "custom"]
        )])
        let estimate = try await jobs.estimate(spec)
        try require(estimate.total == "12.500", "estimate decimal text was not preserved")
        try require(estimate.tasks.first?.mode == .dedicated, "estimate mode was not decoded")

        let encoded = try object(jobs.specification)
        let task = (encoded["tasks"] as! [[String: Any]])[0]
        try require(task["capability"] as? String == "vendor.example/private-model/v7",
                    "custom capability was changed")
        try require((task["capability_filters"] as? [String: String])?["model"] == "custom",
                    "capability filters were not encoded as snake_case")
        try require(task["max_attempts"] as? Int == 3, "Swift task defaults were not encoded")

        let jobID = try await jobs.submit(spec)
        try require(jobID == "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb", "submission ID changed")
        let cancellation = AukiCancellation()
        let keyedID = try await jobs.submitWithKey(spec, idempotencyKey: "persisted-key", cancellation: cancellation)
        try require(keyedID == jobID && jobs.idempotencyKey == "persisted-key", "keyed submission changed key or ID")
        try require(jobs.cancellation === cancellation, "keyed submission lost cancellation")
        let keyedBody = try object(jobs.specification)
        try require(keyedBody["idempotency_key"] == nil, "key belongs in header, not specification")
        let page = try await jobs.list(.init(limit: 10, capabilities: ["vendor.example/private-model/v7"],
                                             matchAllCapabilities: true))
        try require(page.nextCursor == "opaque", "opaque cursor was not decoded")
        let query = try object(jobs.query)
        try require(query["match_all_capabilities"] as? Bool == true, "query was not snake_case")

        try await jobs.close()
        try require(jobs.closeCount == 1, "close was not awaited")
        print("PASS Swift jobs generated bindings and Codable models")
    }
}
