import Foundation
import XCTest
@testable import EnsomiRuntime

/// The chart generator through the Swift API. The model and a release recording are read from beside the
/// repository, where bings-mac keeps them (conformance/generate/README.md); the tests skip without them. The
/// resampler tables are the package's own.
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

        for lowMemory in [false, true] {
            let generator = try paths.open(.cpu, lowMemory: lowMemory)
            let phases = Phases()
            let started = Date()
            let result = try await generator.generate(pcm: pcm, sampleRate: sampleRate, request: request) { phases.append($0) }
            print("GENERATE \(song), low memory \(lowMemory): \(String(format: "%.2f", Date().timeIntervalSince(started))) s")

            XCTAssertNil(firstDifference(result.osu, expected), "the .osu differs from the native chart, low memory \(lowMemory)")
            XCTAssertEqual(result.chart.objects.count, (fixture["objects"] as! NSNumber).intValue)
            XCTAssertEqual(phases.all, [.loading, .timing, .rows, .arrangement, .export])
            XCTAssertEqual(result.receipt.seed, request.seed)
            XCTAssertEqual(result.receipt.modelVersion, "r2-1.0")
            XCTAssertNoThrow(try GameplaySession(chart: result.chart, ruleset: Ruleset(judge: .c)))
        }
    }

    /// The audio stages kept and stored, the whole chart from them, and play while generating: the scheduler puts the
    /// chart into a session page by page and is ready at 0 before the chart is done.
    func testTheSchedulerGeneratesFromStoredAudioStagesAndForPlay() async throws {
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
        let generator = try paths.open(.cpu)
        let lead = LeadPolicy(ruleset: Ruleset(judge: .c), renderLeadMs: 11_485 / 25, marginMs: 0)

        let bytes = try await generator.prepare(pcm: pcm, sampleRate: sampleRate).bytes()
        XCTAssertThrowsError(try Prepared(bytes: bytes.prefix(64)))
        let whole = try ChartScheduler.fromPrepared(
            generator: generator, prepared: Prepared(bytes: bytes), request: request, mode: .wholeChart, lead: lead
        )
        whole.start(atMs: 0, session: GameplaySession.streaming(ruleset: Ruleset(judge: .c)))
        let wholeReady = try await whole.work()
        XCTAssertEqual(wholeReady, .ready(joinMs: 0))
        let wholeDone = try await whole.work()
        XCTAssertEqual(wholeDone, .done)
        let wholeResult = try XCTUnwrap(whole.result())
        XCTAssertNil(firstDifference(wholeResult.osu, expected), "the whole chart from the stored audio stages differs from the native chart")

        let streamed = try ChartScheduler.fromPcm(
            generator: generator, pcm: pcm, sampleRate: sampleRate, request: request, mode: .aheadOfPlay, lead: lead
        )
        let session = GameplaySession.streaming(ruleset: Ruleset(judge: .c))
        streamed.start(atMs: 0, session: session)
        let ready = try await streamed.work()
        XCTAssertEqual(ready, .ready(joinMs: 0))
        XCTAssertLessThan(session.coverageMs(), Double(wholeResult.chart.durationMs))
        let done = try await streamed.work()
        XCTAssertEqual(done, .done)
        let result = try XCTUnwrap(streamed.result())
        XCTAssertEqual(session.coverageMs(), .infinity)
        XCTAssertEqual(session.visibleObjects(lowerMs: -.infinity, upperMs: .infinity).count, result.chart.objects.count)
        XCTAssertEqual(streamed.prepared()?.bytes(), bytes)
        // One arrangement in either mode: the streamed chart is the whole one.
        let receipt = try JSONSerialization.jsonObject(with: Data(result.receipt.receiptJson.utf8)) as! [String: Any]
        let difficulty = (receipt["controls"] as! [String: Any])["difficulty"] as! [String: Any]
        XCTAssertEqual((difficulty["passes"] as! [Any]).count, 1)
        XCTAssertEqual(result.osu, wholeResult.osu)
    }

    /// A request before the first committed page replaces the scheduler's, and the chart is the one a fresh generation
    /// with it makes; once pages are committed, R2 refuses one and the chart goes on unchanged.
    func testARequestChangesTheChartOnlyBeforeTheFirstCommittedPage() async throws {
        let generator = try Paths().open(.cpu)
        let pcm = noise(seconds: 30)
        let changed = GenerateChartRequest(star: 4.5)
        let scheduler = try ChartScheduler.fromPcm(
            generator: generator, pcm: pcm, sampleRate: 22_050, request: GenerateChartRequest(star: 3), mode: .aheadOfPlay,
            lead: LeadPolicy(ruleset: Ruleset(judge: .c), renderLeadMs: 500, marginMs: 0)
        )
        func assertRefused(_ request: GenerateChartRequest) {
            XCTAssertThrowsError(try scheduler.request(request: request)) { error in
                guard case .Generation(let code, _) = error as? GeneratorError else { return XCTFail("\(error)") }
                XCTAssertEqual(code, "unsupported_control")
            }
        }
        assertRefused(GenerateChartRequest(star: 4.5, seed: 1))
        try scheduler.request(request: changed)
        scheduler.start(atMs: 0, session: GameplaySession.streaming(ruleset: Ruleset(judge: .c)))
        let ready = try await scheduler.work()
        XCTAssertEqual(ready, .ready(joinMs: 0))
        assertRefused(GenerateChartRequest(star: 3))
        let done = try await scheduler.work()
        XCTAssertEqual(done, .done)
        let fresh = try await generator.generate(pcm: pcm, sampleRate: 22_050, request: changed)
        XCTAssertEqual(scheduler.result()?.osu, fresh.osu)
    }

    func testModelFilesAreTheGraphsArraysAndNoticesTheConfigNames() throws {
        let paths = try Paths()
        let config = try String(contentsOf: paths.model.appendingPathComponent("runtime.json"), encoding: .utf8)
        let files = try modelFiles(runtimeConfig: config)
        XCTAssertEqual(files.filter { $0.hasSuffix(".onnx") }.count, 6)
        XCTAssertEqual(try noticeFiles(runtimeConfig: config), ["BeatThis-LICENSE.txt"])
        XCTAssertTrue(files.contains("BeatThis-LICENSE.txt"))
        for file in files {
            XCTAssertTrue(FileManager.default.fileExists(atPath: paths.model.appendingPathComponent(file).path), file)
        }
    }

    /// The published package's manifest (conformance/package), read without the model.
    func testCheckPackageReadsThePublishedManifest() throws {
        let manifest = try String(contentsOf: Self.publishedManifest, encoding: .utf8)
        let package = try checkPackage(manifestJson: manifest)
        XCTAssertEqual(package.model, PackageModel(id: "ensomi-r2", name: "Ensomi R2", version: "r2-1.0"))
        XCTAssertEqual(package.notices, ["BeatThis-LICENSE.txt"])
        XCTAssertEqual(package.config, "runtime.json")
        XCTAssertEqual(package.files.count, 30)
        XCTAssertEqual(package.totalBytes, package.files.reduce(0) { $0 + $1.size })
        XCTAssertThrowsError(try checkPackage(manifestJson: manifest.replacingOccurrences(of: "ensomi-package/1", with: "ensomi-package/2"))) { error in
            guard case .Model(let message) = error as? GeneratorError else { return XCTFail("\(error)") }
            XCTAssertTrue(message.contains("ensomi-package/2"), message)
        }
    }

    /// The released files with the published package's manifest beside them: `open` checks the manifest and the files'
    /// sizes, and the capabilities list the manifest's files. A manifest that leaves out a file `runtime.json` names is
    /// refused, and so is a listed file of another size, or missing.
    func testOpenChecksThePackageManifest() throws {
        let paths = try Paths()
        let manifest = try String(contentsOf: Self.publishedManifest, encoding: .utf8)
        let package = try checkPackage(manifestJson: manifest)
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        for file in package.files {
            try FileManager.default.copyItem(at: paths.model.appendingPathComponent(file.path), to: directory.appendingPathComponent(file.path))
        }
        try manifest.write(to: directory.appendingPathComponent("ensomi-package.json"), atomically: true, encoding: .utf8)

        let files = try paths.open(.cpu, directory: directory).capabilities().files
        XCTAssertEqual(files.map(\.path), package.files.map(\.path))
        XCTAssertEqual(files.map(\.sha256Hex), package.files.map(\.sha256))
        XCTAssertEqual(files.map(\.sizeBytes), package.files.map { Optional($0.size) })

        func assertRefused(_ what: String, _ expected: String) {
            XCTAssertThrowsError(try paths.open(.cpu, directory: directory), what) { error in
                guard case .Model(let message) = error as? GeneratorError else { return XCTFail("\(error)") }
                XCTAssertTrue(message.contains(expected), message)
            }
        }
        var unlisted = try JSONSerialization.jsonObject(with: Data(manifest.utf8)) as! [String: Any]
        unlisted["files"] = (unlisted["files"] as! [[String: Any]]).filter { $0["path"] as? String != "r2_release.onnx" }
        try JSONSerialization.data(withJSONObject: unlisted).write(to: directory.appendingPathComponent("ensomi-package.json"))
        assertRefused("a manifest without a graph", "r2_release.onnx is named by runtime.json but not listed in ensomi-package.json")
        try manifest.write(to: directory.appendingPathComponent("ensomi-package.json"), atomically: true, encoding: .utf8)

        let notice = directory.appendingPathComponent("BeatThis-LICENSE.txt")
        let size = "BeatThis-LICENSE.txt is missing or is not the 1113 bytes"
        try Data("another licence".utf8).write(to: notice)
        assertRefused("a file of another size", size)
        try FileManager.default.removeItem(at: notice)
        assertRefused("a missing file", size)
    }

    /// Without a manifest the capabilities list the graphs `runtime.json` names, with no checksums.
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
            pcm: noise(seconds: 5), sampleRate: 22_050, request: GenerateChartRequest(star: 7), listener: nil, cancellation: nil
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

    /// `open` reads the tables from the package's bundle when the options name no directory.
    func testThePackageCarriesTheResamplerTables() throws {
        let files = try FileManager.default.contentsOfDirectory(atPath: XCTUnwrap(ChartGenerator.bundledResamplerDirectory))
        for rate in [11_025, 16_000, 24_000, 32_000, 44_100, 48_000, 88_200, 96_000, 192_000] {
            for name in ["resample-\(rate).json", "resample-\(rate).bin", "resample-\(rate)-boundary.bin"] {
                XCTAssert(files.contains(name), name)
            }
        }
    }

    /// An app finds ONNX Runtime's licence and notices in the package.
    func testThePackageCarriesOnnxRuntimesLicence() throws {
        let files = ChartGenerator.onnxRuntimeLicenseFiles
        XCTAssertEqual(files.map(\.lastPathComponent), ["LICENSE", "ThirdPartyNotices.txt"])
        for file in files {
            XCTAssertFalse(try String(contentsOf: file, encoding: .utf8).isEmpty, file.path)
        }
        XCTAssertTrue(try String(contentsOf: files[0], encoding: .utf8).contains("MIT License"))
    }

    #if os(macOS)
    /// The archive opens with the system's unzip and holds the files the web runtime's `.osz` holds.
    func testOszOpensWithTheChartTheSongAndTheReceipt() throws {
        let osu = "osu file format v14\n\n[General]\nAudioFilename:audio.mp3\nMode: 3\n"
        let audio = Data((0..<50_000).map { UInt8($0 % 251) })
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        let unpacked = directory.appendingPathComponent("unpacked")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let archive = directory.appendingPathComponent("chart.osz")
        try exportOsz(osu: osu, audio: audio, receiptJson: #"{"seed":7}"#).write(to: archive)

        let unzip = Process()
        unzip.executableURL = URL(fileURLWithPath: "/usr/bin/unzip")
        unzip.arguments = ["-q", archive.path, "-d", unpacked.path]
        try unzip.run()
        unzip.waitUntilExit()

        XCTAssertEqual(unzip.terminationStatus, 0)
        XCTAssertEqual(try FileManager.default.contentsOfDirectory(atPath: unpacked.path).sorted(), ["audio.mp3", "generated.osu", "receipt.json"])
        XCTAssertEqual(try String(contentsOf: unpacked.appendingPathComponent("generated.osu"), encoding: .utf8), osu)
        XCTAssertEqual(try Data(contentsOf: unpacked.appendingPathComponent("audio.mp3")), audio)
        XCTAssertEqual(try String(contentsOf: unpacked.appendingPathComponent("receipt.json"), encoding: .utf8), "{\"seed\":7}\n")
    }
    #endif

    // MARK: Inputs

    // .../swift/Tests/EnsomiRuntimeTests/GeneratorTests.swift -> repository root
    private static let root = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent().deletingLastPathComponent()
        .deletingLastPathComponent().deletingLastPathComponent()
    private static let publishedManifest = root.appendingPathComponent("conformance/package/ensomi-package.json")

    private struct Paths {
        let model, recordings, charts: URL

        init() throws {
            let root = GeneratorTests.root
            let workspace = root.deletingLastPathComponent()
            model = workspace.appendingPathComponent("ensomi-model/artifacts/release-r2/bulk/hf/ensomi-r2/onnx")
            recordings = workspace.appendingPathComponent("ensomi-model/artifacts/runtime-ref/bulk")
            charts = root.appendingPathComponent("conformance/generate/native-charts")
            for url in [model, recordings] where !FileManager.default.fileExists(atPath: url.path) {
                throw XCTSkip("\(url.path) is absent")
            }
        }

        func open(_ provider: ExecutionProvider, lowMemory: Bool? = nil, directory: URL? = nil) throws -> ChartGenerator {
            try ChartGenerator.open(
                modelDirectory: (directory ?? model).path,
                options: GeneratorOptions(provider: provider, lowMemory: lowMemory)
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
