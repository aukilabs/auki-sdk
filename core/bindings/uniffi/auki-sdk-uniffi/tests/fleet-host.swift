import Foundation

private enum FleetHostFailure: Error { case assertion(String) }

@main
private struct FleetHost {
    static func require(_ condition: @autoclosure () -> Bool, _ message: String) throws {
        guard condition() else { throw FleetHostFailure.assertion(message) }
    }

    static func main() async throws {
        let endpoint = CommandLine.arguments[1]
        let domain = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"
        let session = try await AukiSession.loginWithEnvironment(
            apiBaseUrl: endpoint, ddsBaseUrl: endpoint, dmsBaseUrl: endpoint + "/v1/",
            email: "fixture@example.test", password: "fixture-password", clientId: "swift-fleet-fixture")
        let fleet = try session.fleet(domainId: domain)
        let inventory = try await fleet.list(.init(capabilities: ["vendor.example/inspect/v7"], matchAllCapabilities: true))
        try require(inventory.complete && inventory.domainId == domain, "incomplete or wrong-Domain inventory")
        try require(inventory.machines.count == 2, "robot pagination did not collect both pages")
        try require(inventory.machines[0].kind == .robot && inventory.machines[0].association == .assigned,
                    "robot assignment not preserved")
        try require(inventory.machines[0].workState == .idle, "covered idle state not decoded")
        let pool = try await fleet.computePool(.init(mode: .dedicated))
        try require(pool.view == .computePool && pool.machines.count == 2, "compute pagination did not collect both pages")
        try require(pool.machines[0].association == .candidate && pool.machines[0].lastSeenAt == nil,
                    "candidate association or unknown last-seen changed")
        let cancellation = AukiCancellation()
        cancellation.cancel()
        do {
            _ = try await fleet.list(cancellation: cancellation)
            throw FleetHostFailure.assertion("cancelled call succeeded")
        } catch AukiSdkError.Fleet(let kind, _, let code, _) {
            try require(kind == "cancelled" && code == "cancelled", "cancellation lost its code")
        }
        try await fleet.close()
        do {
            _ = try await fleet.list()
            throw FleetHostFailure.assertion("closed client succeeded")
        } catch AukiSdkError.Fleet(let kind, _, let code, _) {
            try require(kind == "closed" && code == "closed", "close lost its code")
        }
        let second = try session.fleet(domainId: domain)
        _ = try await second.list()
        try await second.close()
        await session.close()
        print("PASS Swift fleet generated binding, typed models and local HTTP round trip")
    }
}
