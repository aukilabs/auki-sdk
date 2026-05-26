// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "SmokeFullNetwork",
    platforms: [
        .macOS(.v12)
    ],
    dependencies: [
        .package(path: "../../../../../bindings/swift/auki-network")
    ],
    targets: [
        .executableTarget(
            name: "SmokeFullNetwork",
            dependencies: [
                .product(name: "auki_network", package: "auki-network")
            ]
        )
    ]
)
