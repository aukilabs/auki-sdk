// swift-tools-version: 6.0

import PackageDescription

let package = Package(
    name: "AukiPortableEcho",
    platforms: [
        .iOS(.v17),
    ],
    products: [
        .library(name: "AukiPortableEcho", targets: ["AukiPortableEcho"]),
    ],
    targets: [
        .binaryTarget(
            name: "AukiPortableEchoFFI",
            path: "Artifacts/AukiPortableEcho.xcframework"
        ),
        .target(
            name: "AukiPortableEcho",
            dependencies: ["AukiPortableEchoFFI"],
            path: "Sources/AukiPortableEcho",
            linkerSettings: [
                .linkedFramework("SystemConfiguration"),
                .linkedFramework("CoreFoundation"),
                .linkedLibrary("iconv"),
            ]
        ),
    ]
)
