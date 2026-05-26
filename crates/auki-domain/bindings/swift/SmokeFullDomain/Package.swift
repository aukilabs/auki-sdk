// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "SmokeFullDomain",
    platforms: [
        .macOS(.v12)
    ],
    dependencies: [
        .package(path: "../../../../../bindings/swift/auki-network"),
        .package(path: "../../../../../bindings/swift/auki-domain")
    ],
    targets: [
        .executableTarget(
            name: "SmokeFullDomain",
            dependencies: [
                .product(name: "auki_network", package: "auki-network"),
                .product(name: "auki_domain", package: "auki-domain")
            ]
        )
    ]
)
