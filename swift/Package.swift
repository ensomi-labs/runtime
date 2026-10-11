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
        // The xcframework carries ONNX Runtime, which is C++ and uses Core ML.
        .target(
            name: "EnsomiRuntime",
            dependencies: ["EnsomiRuntimeFFI"],
            linkerSettings: [.linkedLibrary("c++"), .linkedFramework("CoreML"), .linkedFramework("Foundation")]
        ),
        .testTarget(name: "EnsomiRuntimeTests", dependencies: ["EnsomiRuntime"]),
    ],
    swiftLanguageModes: [.v5]
)
