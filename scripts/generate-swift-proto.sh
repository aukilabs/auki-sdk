#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
pkg="bindings/swift/auki-proto"
src="$pkg/Sources/AukiProto"

command -v protoc-gen-swift >/dev/null 2>&1 || {
  echo "protoc-gen-swift is required. Install SwiftProtobuf's protoc plugin." >&2
  exit 1
}

rm -rf "$pkg"
mkdir -p "$src"

cat > "$pkg/Package.swift" <<'SWIFT'
// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "auki-proto",
    platforms: [.iOS(.v13), .macOS(.v12)],
    products: [
        .library(name: "AukiProto", targets: ["AukiProto"])
    ],
    dependencies: [
        .package(url: "https://github.com/apple/swift-protobuf.git", from: "1.38.0")
    ],
    targets: [
        .target(
            name: "AukiProto",
            dependencies: [.product(name: "SwiftProtobuf", package: "swift-protobuf")]
        )
    ]
)
SWIFT

cat > "$pkg/README.md" <<'MD'
# AukiProto

Generated SwiftProtobuf bindings from root `proto/auki` schemas.

This directory is generated locally by `scripts/generate-swift-proto.sh` and is ignored by git. Do not edit or commit generated files from this directory.
MD

protoc \
  -I proto \
  --swift_out "$src" \
  --swift_opt Visibility=Public \
  proto/auki/*.proto
