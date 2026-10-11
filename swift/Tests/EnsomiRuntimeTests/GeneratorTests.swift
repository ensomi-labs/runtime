import Foundation
import XCTest
@testable import EnsomiRuntime

/// The chart generator through the Swift API. The model, a release recording and the resampler tables are read
/// from beside the repository, where bings-mac keeps them (conformance/generate/README.md); the tests skip
/// without them.
final class GeneratorTests: XCTestCase {
    /// The chart the native driver made from the same PCM, request and seed: conformance/generate/native-charts.
    func testGeneratesTheNativeChartFromRecordedPCM() async throws {
        let paths = try Paths()
        let song = "hesitation-snow"
        let fixture = try JSONSerialization.jsonObject(
            with: Data(contentsOf: paths.charts.appendingPathComponent("\(song).json"))
        ) as! [String: Any]
        let expected = try String(contentsOf: paths.charts.appendingPathComponent("\(song).osu"), encoding: .utf8)
        let recorded = fixture["request"] as! [String: Any]
        let request = GenerateChartRequest(
            star: (recorded["star"] as! NSNumber).doubleValue,
            seed: (fixture["seed"] as! NSNumber).uint64Value,
            title: recorded["title"] as? String,
            artist: recorded["artist"] as? String
        )
        let (pcm, sampleRate) = try recordedPCM(paths.recordings.appendingPathComponent(song))
        XCTAssertEqual(sampleRate, (fixture["sample_rate"] as! NSNumber).doubleValue)

        let generator = try paths.open(.cpu)
        let phases = Phases()
        let result = try await generator.generate(pcm: pcm, sampleRate: sampleRate, request: request) { phases.append($0) }

        XCTAssertNil(firstDifference(result.osu, expected), "the .osu differs from the native chart")
        XCTAssertEqual(result.chart.objects.count, (fixture["objects"] as! NSNumber).intValue)
        XCTAssertEqual(phases.all, [.loading, .timing, .rows, .arrangement, .export])
        XCTAssertEqual(result.receipt.seed, request.seed)
        XCTAssertEqual(result.receipt.modelVersion, "r2-1.0")
        XCTAssertNoThrow(try GameplaySession(chart: result.chart, ruleset: Ruleset(judge: .c)))
    }

    func testCapabilitiesDescribeTheModel() throws {
        let capabilities = try Paths().open(.cpu).capabilities()
        XCTAssertEqual(capabilities.modelName, "Ensomi R2")
        XCTAssertEqual(capabilities.minStar, 2)
        XCTAssertEqual(capabilities.maxStarExclusive, 6)
        XCTAssertEqual(capabilities.files.count, 6)
        XCTAssertEqual(capabilities.controls.first { $0.name == "star" }?.status, .works)
        XCTAssertEqual(capabilities.controls.first { $0.name == "chordjack" }?.status, .notWorking)
    }

    func testRefusedRequestCarriesItsProtocolCode() throws {
        let generator = try Paths().open(.cpu)
        XCTAssertThrowsError(try generator.generate(
            pcm: bytes(noise(seconds: 5)), sampleRate: 22_050, request: GenerateChartRequest(star: 7), listener: nil, cancellation: nil
        )) { error in
            guard case .Generation(let code, _) = error as? GeneratorError else { return XCTFail("\(error)") }
            XCTAssertEqual(code, "star_out_of_range")
        }
    }

    func testCancellingTheTaskStopsTheGeneration() async throws {
        let generator = try Paths().open(.cpu)
        let timing = expectation(description: "timing phase")
        let task = Task {
            try await generator.generate(pcm: noise(seconds: 90), sampleRate: 22_050, request: GenerateChartRequest(star: 3)) { phase in
                if phase == .timing { timing.fulfill() }
            }
        }
        await fulfillment(of: [timing], timeout: 60)
        task.cancel()
        do {
            _ = try await task.value
            XCTFail("the generation finished")
        } catch GeneratorError.Cancelled {}
    }

    // MARK: Inputs

    private struct Paths {
        let model, recordings, resampler, charts: URL

        init() throws {
            // .../swift/Tests/EnsomiRuntimeTests/GeneratorTests.swift -> repository root
            let root = URL(fileURLWithPath: #filePath)
                .deletingLastPathComponent().deletingLastPathComponent()
                .deletingLastPathComponent().deletingLastPathComponent()
            let workspace = root.deletingLastPathComponent()
            model = workspace.appendingPathComponent("ensomi-model/artifacts/release-r2/bulk/hf/ensomi-r2/onnx")
            recordings = workspace.appendingPathComponent("ensomi-model/artifacts/runtime-ref/bulk")
            resampler = workspace.appendingPathComponent("ensomi-web/packages/runtime/assets")
            charts = root.appendingPathComponent("conformance/generate/native-charts")
            for url in [model, recordings, resampler] where !FileManager.default.fileExists(atPath: url.path) {
                throw XCTSkip("\(url.path) is absent")
            }
        }

        func open(_ provider: ExecutionProvider) throws -> ChartGenerator {
            try ChartGenerator.open(
                modelDirectory: model.path, options: GeneratorOptions(provider: provider, resamplerDirectory: resampler.path)
            )
        }
    }

    /// The `audio` event's PCM of an `ensomi-runtime-reference-v1` recording with its channels averaged in
    /// Double and rounded to Float, as the native driver made the reference chart's input.
    private func recordedPCM(_ directory: URL) throws -> ([Float], Double) {
        let index = try JSONSerialization.jsonObject(with: Data(contentsOf: directory.appendingPathComponent("index.json"))) as! [String: Any]
        let arrays = index["arrays"] as! [[String: Any]]
        let event = (index["events"] as! [[String: Any]]).first { $0["name"] as? String == "audio" }!
        let ids = event["arrays"] as! [String: Int]
        let file = try FileHandle(forReadingFrom: directory.appendingPathComponent("arrays.bin"))
        defer { try? file.close() }
        func read(_ key: String, dtype: String) throws -> (Data, [Int]) {
            let entry = arrays[ids[key]!]
            XCTAssertEqual(entry["dtype"] as? String, dtype)
            try file.seek(toOffset: (entry["byte_offset"] as! NSNumber).uint64Value)
            return (try file.read(upToCount: (entry["nbytes"] as! NSNumber).intValue)!, entry["shape"] as! [Int])
        }
        let (rate, _) = try read("sample_rate", dtype: "<i8")
        let (samples, shape) = try read("pcm", dtype: "<f8")
        let channels = shape.count > 1 ? shape[1] : 1
        let pcm = samples.withUnsafeBytes { raw -> [Float] in
            let values = raw.bindMemory(to: Double.self)
            return (0..<values.count / channels).map { frame in
                var sum = 0.0
                for channel in 0..<channels { sum += values[frame * channels + channel] }
                return Float(sum / Double(channels))
            }
        }
        return (pcm, Double(rate.withUnsafeBytes { $0.loadUnaligned(as: Int64.self) }))
    }

    /// Reproducible noise at 22050 Hz, the rate BeatThis reads without a resampler table.
    private func noise(seconds: Int) -> [Float] {
        var state: UInt32 = 1
        return (0..<seconds * 22_050).map { _ in
            state = state &* 1_664_525 &+ 1_013_904_223
            return Float(state >> 8) / Float(1 << 24) - 0.5
        }
    }

    private func bytes(_ pcm: [Float]) -> Data {
        pcm.withUnsafeBufferPointer { Data(buffer: $0) }
    }

    private func firstDifference(_ actual: String, _ expected: String) -> String? {
        let (a, b) = (actual.components(separatedBy: "\n"), expected.components(separatedBy: "\n"))
        guard let line = (0..<max(a.count, b.count)).first(where: { $0 >= a.count || $0 >= b.count || a[$0] != b[$0] }) else {
            return nil
        }
        return "line \(line + 1): \(line < a.count ? a[line] : "<end>") vs \(line < b.count ? b[line] : "<end>")"
    }
}

private final class Phases: @unchecked Sendable {
    private let lock = NSLock()
    private var phases: [GenerationPhase] = []

    func append(_ phase: GenerationPhase) {
        lock.withLock { phases.append(phase) }
    }

    var all: [GenerationPhase] {
        lock.withLock { phases }
    }
}
