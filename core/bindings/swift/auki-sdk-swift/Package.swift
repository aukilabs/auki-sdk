// swift-tools-version: 6.0

import PackageDescription

let package = Package(
    name: "AukiSDK",
    platforms: [
        .iOS(.v17),
    ],
    products: [
        .library(name: "AukiSDK", targets: ["AukiSDK"]),
    ],
    targets: [
        .binaryTarget(
            name: "AukiSDKFFI",
            path: "target-xcframework/AukiSDK.xcframework"
        ),
        .target(
            name: "AukiSDK",
            dependencies: ["AukiSDKFFI"],
            path: "Sources/AukiSDK",
            linkerSettings: [
                .linkedFramework("SystemConfiguration"),
                .linkedFramework("CoreFoundation"),
                .linkedLibrary("iconv"),
            ]
        ),
    ]
)
