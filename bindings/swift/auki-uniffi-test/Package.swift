// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "auki-uniffi-test",
    platforms: [
        .iOS(.v13),
        .macOS(.v12)
    ],
    products: [
        .library(name: "auki_uniffi_test", targets: ["auki_uniffi_test"])
    ],
    targets: [
        .target(
            name: "auki_uniffi_test",
            dependencies: ["auki_uniffi_testFFI"],
            path: "generated",
            sources: ["auki_uniffi_test.swift"]
        ),
        .binaryTarget(
            name: "auki_uniffi_testFFI",
            path: "generated/auki_uniffi_test.xcframework"
        )
    ]
)
