// swift-tools-version: 6.0
import Foundation
import PackageDescription

// The release workflow writes these when it publishes the xcframework (RELEASING.md).
private let releaseURL = "https://github.com/ensomi-labs/runtime/releases/download/v0.3.0/EnsomiRuntimeFFI.xcframework.zip"
private let releaseChecksum = "79b8861d273f9daf8b58ae96583b323b3b926b6a0f6bc0ed4c7055257b0b2f22"

// ENSOMI_RUNTIME_XCFRAMEWORK selects a local xcframework instead, such as the
// build/apple/EnsomiRuntimeFFI.xcframework that scripts/build-apple.sh builds.
let binary: Target
if let path = ProcessInfo.processInfo.environment["ENSOMI_RUNTIME_XCFRAMEWORK"], !path.isEmpty {
    // SwiftPM takes a binary target's path relative to the package; the variable may be absolute.
    let root = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
    let artifact = (path.hasPrefix("/") ? URL(fileURLWithPath: path) : root.appendingPathComponent(path)).standardizedFileURL.pathComponents
    let base = root.standardizedFileURL.pathComponents
    let shared = zip(base, artifact).prefix(while: { $0 == $1 }).count
    let relative = (Array(repeating: "..", count: base.count - shared) + artifact.dropFirst(shared)).joined(separator: "/")
    binary = .binaryTarget(name: "EnsomiRuntimeFFI", path: relative)
} else {
    binary = .binaryTarget(name: "EnsomiRuntimeFFI", url: releaseURL, checksum: releaseChecksum)
}

let package = Package(
    name: "EnsomiRuntime",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [
        .library(name: "EnsomiRuntime", targets: ["EnsomiRuntime"]),
    ],
    targets: [
        binary,
        // The xcframework carries ONNX Runtime, which is C++ and uses Core ML; its licence and notices are in
        // licenses/onnxruntime. The resampler tables are the generator's audio front end, shared with the npm package.
        .target(
            name: "EnsomiRuntime",
            dependencies: ["EnsomiRuntimeFFI"],
            path: "swift/Sources/EnsomiRuntime",
            resources: [.copy("../../../assets/resampler"), .copy("../../../licenses")],
            linkerSettings: [.linkedLibrary("c++"), .linkedFramework("CoreML"), .linkedFramework("Foundation")]
        ),
        .testTarget(name: "EnsomiRuntimeTests", dependencies: ["EnsomiRuntime"], path: "swift/Tests/EnsomiRuntimeTests"),
    ],
    swiftLanguageModes: [.v5]
)
