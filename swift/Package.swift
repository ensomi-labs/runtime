// swift-tools-version: 6.0
import PackageDescription

// The binary target is built by scripts/build-apple.sh into build/apple (not checked in).
let package = Package(
    name: "EnsomiRuntime",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [
        .library(name: "EnsomiRuntime", targets: ["EnsomiRuntime"]),
    ],
    targets: [
        .binaryTarget(name: "EnsomiRuntimeFFI", path: "../build/apple/EnsomiRuntimeFFI.xcframework"),
        .target(name: "EnsomiRuntime", dependencies: ["EnsomiRuntimeFFI"]),
        .testTarget(name: "EnsomiRuntimeTests", dependencies: ["EnsomiRuntime"]),
    ],
    swiftLanguageModes: [.v5]
)
