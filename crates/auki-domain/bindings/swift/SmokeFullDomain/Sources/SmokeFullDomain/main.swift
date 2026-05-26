import Darwin
import Foundation
import auki_domain
import auki_network

@main
enum SmokeFullDomain {
    static func main() async throws {
        let seed = Data(repeating: 51, count: 32)
        let clusterName = "swift-smoke"
        let server = try MockDiscoveryServer(
            clusterName: clusterName,
            managerPeerId: try peerIdFromWalletSeed(seed: seed)
        )
        let multiaddr = try reserveLocalMultiaddr()

        let manager = try await bootstrapDomainClusterManager(
            targetMode: .create,
            targetName: clusterName,
            walletSeed: seed,
            listenAddrs: [multiaddr],
            advertiseMultiaddrs: [multiaddr],
            discoveryUrl: server.baseUrl,
            daemonInfo: DaemonInfo(
                app: "swift-smoke",
                name: "peer-51",
                sessionId: "session-51",
                sessionClockId: "legacy-clock",
                sessionClockHash: "legacy-clock-hash",
                appInstance: "00163eabcdef"
            ),
            agentVersion: "auki-domain-swift-smoke/0.1"
        )

        defer {
            server.stop()
        }

        precondition(!manager.localPeerId().isEmpty, "domain local peer id is empty")
        let membership = try manager.membershipJson()
        precondition(membership.contains("\"cluster_name\":\"\(clusterName)\""), "membership JSON missing cluster")
        try await manager.shutdown()
    }
}

private final class MockDiscoveryServer {
    private let listener: Int32
    private let clusterName: String
    private let managerPeerId: String
    let port: UInt16

    var baseUrl: String {
        "http://127.0.0.1:\(port)"
    }

    init(clusterName: String, managerPeerId: String) throws {
        self.clusterName = clusterName
        self.managerPeerId = managerPeerId
        let listenerFd = Darwin.socket(AF_INET, SOCK_STREAM, 0)
        guard listenerFd >= 0 else {
            throw SmokeError.socket("socket")
        }
        self.listener = listenerFd

        var reuse: Int32 = 1
        guard Darwin.setsockopt(
            listenerFd,
            SOL_SOCKET,
            SO_REUSEADDR,
            &reuse,
            socklen_t(MemoryLayout<Int32>.size)
        ) == 0 else {
            throw SmokeError.socket("setsockopt")
        }

        var address = sockaddr_in()
        address.sin_family = sa_family_t(AF_INET)
        address.sin_port = in_port_t(0).bigEndian
        address.sin_addr = in_addr(s_addr: inet_addr("127.0.0.1"))

        let bindResult = withUnsafePointer(to: &address) { pointer in
            pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) { sockaddrPointer in
                Darwin.bind(listenerFd, sockaddrPointer, socklen_t(MemoryLayout<sockaddr_in>.size))
            }
        }
        guard bindResult == 0 else {
            throw SmokeError.socket("bind")
        }
        guard Darwin.listen(listenerFd, 16) == 0 else {
            throw SmokeError.socket("listen")
        }

        var bound = sockaddr_in()
        var length = socklen_t(MemoryLayout<sockaddr_in>.size)
        let sockNameResult = withUnsafeMutablePointer(to: &bound) { pointer in
            pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) { sockaddrPointer in
                Darwin.getsockname(listenerFd, sockaddrPointer, &length)
            }
        }
        guard sockNameResult == 0 else {
            throw SmokeError.socket("getsockname")
        }
        self.port = UInt16(bigEndian: bound.sin_port)

        let clusterName = self.clusterName
        let managerPeerId = self.managerPeerId
        Thread.detachNewThread {
            Self.acceptLoop(listener: listenerFd, clusterName: clusterName, managerPeerId: managerPeerId)
        }
    }

    func stop() {
        _ = Darwin.shutdown(listener, SHUT_RDWR)
        _ = Darwin.close(listener)
    }

    private static func acceptLoop(listener: Int32, clusterName: String, managerPeerId: String) {
        while true {
            let client = Darwin.accept(listener, nil, nil)
            if client < 0 {
                return
            }
            handle(client: client, clusterName: clusterName, managerPeerId: managerPeerId)
            _ = Darwin.close(client)
        }
    }

    private static func handle(client: Int32, clusterName: String, managerPeerId: String) {
        var noSigpipe: Int32 = 1
        _ = Darwin.setsockopt(
            client,
            SOL_SOCKET,
            SO_NOSIGPIPE,
            &noSigpipe,
            socklen_t(MemoryLayout<Int32>.size)
        )

        let request = readRequest(client: client)
        let firstLine = request.components(separatedBy: "\r\n").first ?? ""
        let parts = firstLine.split(separator: " ")
        let method = parts.indices.contains(0) ? String(parts[0]) : ""
        let path = parts.indices.contains(1) ? String(parts[1]) : ""
        let entry = discoveryEntry(clusterName: clusterName, managerPeerId: managerPeerId)

        if method == "POST" && path == "/clusters/\(clusterName)" {
            writeResponse(client: client, status: "201 Created", body: entry)
        } else if method == "DELETE" && path == "/clusters/\(clusterName)" {
            writeResponse(client: client, status: "204 No Content", body: "")
        } else if method == "GET" && path == "/clusters/\(clusterName)/liveness" {
            writeResponse(client: client, status: "200 OK", body: entry)
        } else if method == "GET" && path == "/clusters" {
            writeResponse(client: client, status: "200 OK", body: "{\"clusters\":[\(entry)]}")
        } else {
            writeResponse(client: client, status: "404 Not Found", body: "{\"error\":\"unexpected path\"}")
        }
    }

    private static func readRequest(client: Int32) -> String {
        var data = Data()
        var buffer = [UInt8](repeating: 0, count: 1024)
        while true {
            let count = Darwin.recv(client, &buffer, buffer.count, 0)
            if count <= 0 {
                break
            }
            data.append(buffer, count: count)
            if data.range(of: Data("\r\n\r\n".utf8)) != nil {
                break
            }
        }
        return String(decoding: data, as: UTF8.self)
    }

    private static func writeResponse(client: Int32, status: String, body: String) {
        let response = """
        HTTP/1.1 \(status)\r
        content-type: application/json\r
        content-length: \(body.utf8.count)\r
        connection: close\r
        \r
        \(body)
        """
        response.withCString { pointer in
            _ = Darwin.send(client, pointer, strlen(pointer), 0)
        }
    }

    private static func discoveryEntry(clusterName: String, managerPeerId: String) -> String {
        """
        {"name":"\(clusterName)","manager_peer_id":"\(managerPeerId)","manager_multiaddrs":["/ip4/127.0.0.1/tcp/48000"],"peer_count":1,"created_ns":1,"last_liveness_check_ns":1}
        """
    }
}

private func reserveLocalMultiaddr() throws -> String {
    let socketFd = Darwin.socket(AF_INET, SOCK_STREAM, 0)
    guard socketFd >= 0 else {
        throw SmokeError.socket("socket")
    }
    defer {
        _ = Darwin.close(socketFd)
    }

    var address = sockaddr_in()
    address.sin_family = sa_family_t(AF_INET)
    address.sin_port = in_port_t(0).bigEndian
    address.sin_addr = in_addr(s_addr: inet_addr("127.0.0.1"))

    let bindResult = withUnsafePointer(to: &address) { pointer in
        pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) { sockaddrPointer in
            Darwin.bind(socketFd, sockaddrPointer, socklen_t(MemoryLayout<sockaddr_in>.size))
        }
    }
    guard bindResult == 0 else {
        throw SmokeError.socket("bind")
    }

    var bound = sockaddr_in()
    var length = socklen_t(MemoryLayout<sockaddr_in>.size)
    let sockNameResult = withUnsafeMutablePointer(to: &bound) { pointer in
        pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) { sockaddrPointer in
            Darwin.getsockname(socketFd, sockaddrPointer, &length)
        }
    }
    guard sockNameResult == 0 else {
        throw SmokeError.socket("getsockname")
    }

    return "/ip4/127.0.0.1/tcp/\(UInt16(bigEndian: bound.sin_port))"
}

private enum SmokeError: Error {
    case socket(String)
}
