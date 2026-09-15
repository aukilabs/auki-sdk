import Foundation

private enum HostFailure: Error, CustomStringConvertible {
    case assertion(String)

    var description: String {
        switch self {
        case .assertion(let message): return message
        }
    }
}

private actor RetryZitadelStore: AukiZitadelSessionStore {
    private var failNext = true
    private var snapshots: [[String]] = []

    func save(credentials: AukiZitadelCredentials) async throws {
        snapshots.append([
            credentials.exposeAccessToken(),
            credentials.exposeRefreshToken(),
            credentials.accessTokenExpiresAt() ?? "",
        ])
        if failNext {
            failNext = false
            throw AukiPersistenceError.Failed
        }
    }

    func savedSnapshots() -> [[String]] { snapshots }
}

@main
struct DomainDataHost {
    private static let domainID = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"
    private static let initialDataID = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"
    private static let portalID = "eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee"
    private static let portalShortID = "ABC12345678"
    private static let deniedDataID = "ffffffff-ffff-4fff-8fff-ffffffffffff"

    private static func require(_ condition: @autoclosure () -> Bool, _ message: String) throws {
        guard condition() else { throw HostFailure.assertion(message) }
    }

    private static func expectDenied(_ operation: () async throws -> Void) async throws {
        do {
            try await operation()
            throw HostFailure.assertion("expected Domain data permission denial")
        } catch AukiSdkError.DomainData(let kind, let status, _, _) {
            try require(kind == .http && status == 403, "permission error lost kind or HTTP status")
        }
    }

    private static func forceOneDDSRenewal(_ baseURL: String) async throws {
        guard let url = URL(string: "\(baseURL)/__configure") else {
            throw HostFailure.assertion("invalid loopback fixture URL")
        }
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "content-type")
        request.httpBody = Data(#"{"ddsUnauthorizedOnce":true}"#.utf8)
        let (_, response) = try await URLSession.shared.data(for: request)
        try require((response as? HTTPURLResponse)?.statusCode == 200, "failed to configure loopback renewal")
    }

    static func main() async throws {
        guard CommandLine.arguments.count == 2 else {
            throw HostFailure.assertion("usage: domain-data-host <loopback-base-url>")
        }
        let baseURL = CommandLine.arguments[1]

        let session = try await AukiSession.loginWithEnvironment(
            apiBaseUrl: baseURL,
            ddsBaseUrl: baseURL,
            dmsBaseUrl: baseURL,
            email: "swift@example.invalid",
            password: "fixture-password",
            clientId: "swift-domain-data-host"
        )

        do {
            try await forceOneDDSRenewal(baseURL)
            let domains = session.domains()
            let page = try await domains.list(
                query: AukiDomainListQuery(limit: 1, offset: 0)
            )
            try require(page.total == 1 && page.domains.first?.id == domainID, "Domain page was not preserved")

            let portalDomains = try await domains.forPortal(portal: portalShortID)
            try require(portalDomains.first?.id == domainID, "portal Domain association was not preserved")
            let portals = try await domains.portals(domainId: domainID)
            try require(portals.first?.id == portalID, "Domain portals were not preserved")
            let portal = try await domains.portal(domainId: domainID, portal: portalShortID)
            try require(portal.shortId == portalShortID, "short portal ID lookup failed")

            let data = try session.data(domainId: domainID)
            do {
                let records = try await data.list(query: AukiDataListQuery(dataType: "fixture.v1"))
                try require(records.first?.id == initialDataID, "Domain data metadata list was not preserved")
                let initialMetadata = try await data.get(dataId: initialDataID)
                try require(initialMetadata.size == 7, "Domain data get returned the wrong size")

                let download = try await data.startDownload(
                    dataId: initialDataID,
                    options: AukiTransferOptions(maxBytes: 32, maxChunkBytes: 8)
                )
                var downloaded = Data()
                while let chunk = try await download.next() {
                    try require(chunk.count <= 8, "download exceeded its Swift chunk bound")
                    downloaded.append(chunk)
                }
                try await download.close()
                try require(downloaded == Data("fixture".utf8), "streaming download changed bytes")

                try await expectDenied {
                    _ = try await data.get(dataId: deniedDataID)
                }

                let direct = Data("direct".utf8)
                let directMetadata = try await data.write(
                    target: .named(name: "swift-direct", dataType: "fixture.swift.v1"),
                    bytes: direct
                )
                let directRoundTrip = try await data.read(dataId: directMetadata.id)
                try require(directRoundTrip == direct, "direct write did not round trip")
                try await data.delete(dataId: directMetadata.id)

                let uploadedBytes = Data("1234567".utf8)
                let upload = try await data.startUpload(
                    target: .named(name: "swift-stream", dataType: "fixture.swift.v1"),
                    size: UInt64(uploadedBytes.count),
                    options: AukiTransferOptions(maxBytes: 32, maxChunkBytes: 4)
                )
                var offset = 0
                while let maximum = try await upload.nextMaximum() {
                    if offset == uploadedBytes.count {
                        try await upload.push(bytes: Data())
                        continue
                    }
                    let count = min(Int(maximum), uploadedBytes.count - offset)
                    try await upload.push(bytes: uploadedBytes.subdata(in: offset..<(offset + count)))
                    offset += count
                }
                let uploaded = try await upload.result()
                try await upload.close()
                let uploadRoundTrip = try await data.read(dataId: uploaded.id)
                try require(uploadRoundTrip == uploadedBytes, "multipart upload did not round trip")
                try await data.delete(dataId: uploaded.id)

                let cancelled = try await data.startUpload(
                    target: .named(name: "swift-cancelled", dataType: "fixture.swift.v1"),
                    size: 7,
                    options: AukiTransferOptions(maxBytes: 32, maxChunkBytes: 4)
                )
                guard let firstMaximum = try await cancelled.nextMaximum() else {
                    throw HostFailure.assertion("cancelled upload ended before requesting data")
                }
                try await cancelled.push(bytes: Data("abc".utf8).prefix(Int(firstMaximum)))
                cancelled.cancel()
                try await cancelled.close()

                let poses = try await data.poses()
                try require(poses.first?.id == portalID, "portal poses were not preserved")
                let pose = try await data.pose(portal: portalShortID)
                try require(pose.domainId == domainID && pose.px == 1, "portal pose lookup changed fields")

                try await data.close()
                do {
                    _ = try await data.list(query: AukiDataListQuery())
                    throw HostFailure.assertion("closed Domain data client accepted an operation")
                } catch AukiSdkError.DomainData(let kind, _, _, _) {
                    try require(kind == .closed, "closed client lost its structured error kind")
                }
            } catch {
                try? await data.close()
                throw error
            }
        } catch {
            await session.close()
            throw error
        }
        await session.close()

        let httpOnly = try await AukiSession.loginDataWithEnvironment(
            apiBaseUrl: baseURL,
            ddsBaseUrl: baseURL,
            email: "swift@example.invalid",
            password: "fixture-password"
        )
        await httpOnly.close()

        let importedStore = RetryZitadelStore()
        let importedCredentials = try AukiZitadelCredentials(
            accessToken: "imported-access-0",
            refreshToken: "imported-refresh-0",
            clientId: "domain-data-public-client",
            issuer: baseURL,
            accessTokenExpiresAt: "2000-01-01T00:00:00Z"
        )
        let imported = try AukiSession.importZitadelDataWithEnvironment(
            apiBaseUrl: baseURL,
            ddsBaseUrl: baseURL,
            credentials: importedCredentials,
            store: importedStore,
            clientId: "swift-imported-domain-data-host"
        )
        let importedData = try imported.data(domainId: domainID)
        do {
            _ = try await importedData.read(dataId: initialDataID)
            throw HostFailure.assertion("failed imported credential save allowed Domain data read")
        } catch AukiSdkError.DomainData(let kind, _, let authKind, _) {
            try require(
                kind == .authentication && authKind == .persistence,
                "imported save failure lost its structured persistence code"
            )
        }
        let importedBytes = try await importedData.read(dataId: initialDataID)
        try require(importedBytes == Data("fixture".utf8), "imported session could not read known-Domain data")
        do {
            _ = try await imported.domains().list(query: AukiDomainListQuery())
            throw HostFailure.assertion("imported session unexpectedly listed Domains")
        } catch AukiSdkError.DomainData(_, _, let authKind, _) {
            try require(authKind == .configuration, "imported listing lost provider limitation code")
        }
        try await importedData.close()
        await imported.close()
        let savedSnapshots = await importedStore.savedSnapshots()
        try require(
            savedSnapshots.count == 2 && savedSnapshots[0] == savedSnapshots[1],
            "imported persistence retry rotated or lost retained credentials"
        )

        print("PASS generated Swift Domain discovery/data FFI runtime")
    }
}
