// swift-tools-version: 6.0

import PackageDescription

let package = Package(
  name: "AukiCameraMesh",
  platforms: [
    .iOS(.v17)
  ],
  products: [
    .library(name: "AukiCameraMesh", targets: ["AukiCameraMesh"])
  ],
  dependencies: [],
  targets: [
    .binaryTarget(
      name: "AukiCameraMeshFFI",
      path: "Artifacts/AukiCameraMesh.xcframework"
    ),
    .target(
      name: "AukiCameraMesh",
      dependencies: ["AukiCameraMeshFFI"],
      linkerSettings: [
        .linkedFramework("SystemConfiguration"),
        .linkedFramework("CoreFoundation"),
        .linkedLibrary("iconv"),
      ]
    ),
    .testTarget(
      name: "AukiCameraMeshTests",
      dependencies: ["AukiCameraMesh"]
    ),
  ]
)
