// swift-tools-version: 6.0

import PackageDescription

let package = Package(
    name: "AukiStandardPlayground",
    platforms: [
        .iOS(.v17),
    ],
    products: [
        .library(name: "AukiStandardPlayground", targets: ["AukiStandardPlayground"]),
    ],
    dependencies: [
        .package(path: "../../../../../core/bindings/swift/auki-sdk-swift"),
    ],
    targets: [
        .target(
            name: "AukiStandardPlayground",
            dependencies: [
                .product(name: "AukiSDK", package: "auki-sdk-swift"),
            ]
        ),
    ]
)
