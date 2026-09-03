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
  dependencies: [
    .package(path: "../../../../bindings/swift/auki-sdk-swift")
  ],
  targets: [
    .target(
      name: "AukiCameraMesh",
      dependencies: [
        .product(name: "AukiSDK", package: "auki-sdk-swift")
      ]
    ),
    .testTarget(
      name: "AukiCameraMeshTests",
      dependencies: [
        "AukiCameraMesh",
        .product(name: "AukiSDK", package: "auki-sdk-swift"),
      ]
    ),
  ]
)
