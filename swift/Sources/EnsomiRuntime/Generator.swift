import Foundation

extension ChartGenerator {
    /// The package's resource bundle. Xcode copies it into the app or test bundle that links the package, where
    /// `Bundle.module`, which aborts when it finds nothing, does not look if a framework links the package; so the
    /// loaded bundles come first.
    private static let resources: Bundle = {
        let copies = (Bundle.allBundles + Bundle.allFrameworks).lazy.compactMap { bundle in
            bundle.resourceURL.flatMap { Bundle(url: $0.appendingPathComponent("EnsomiRuntime_EnsomiRuntime.bundle")) }
        }
        return copies.first ?? Bundle.module
    }()

    /// The resampler tables the package carries: `resample-<rate>.json`, `.bin` and `-boundary.bin`.
    static let bundledResamplerDirectory: String? = resources.url(forResource: "resampler", withExtension: nil)?.path

    /// ONNX Runtime's licence (MIT) and third-party notices, `LICENSE` and `ThirdPartyNotices.txt`, which the package
    /// carries: an app that ships the generator, and with it ONNX Runtime, shows them with its acknowledgements.
    public static let onnxRuntimeLicenseFiles: [URL] = ["LICENSE", "ThirdPartyNotices.txt"].compactMap {
        resources.url(forResource: $0, withExtension: nil, subdirectory: "licenses/onnxruntime")
    }

    /// Opens a model directory. With an `ensomi-package.json`, checks it (`checkPackage`), requires every file it lists
    /// at its listed size, reads the runtime config it names and requires the manifest to list every file that config
    /// names; the SHA-256s are the downloader's check. Without it, reads `runtime.json` and the files it names. Without
    /// `options.resamplerDirectory`, PCM at a rate other than 22050 Hz is resampled with the tables the package carries.
    public static func open(modelDirectory: String, options: GeneratorOptions) throws -> ChartGenerator {
        var options = options
        options.resamplerDirectory = options.resamplerDirectory ?? bundledResamplerDirectory
        return try ffiOpen(modelDirectory: modelDirectory, options: options)
    }

    /// Generates a chart from mono PCM at `sampleRate` on a thread of its own. `progress` hears each phase on
    /// that thread. Cancelling the task stops the generation at its next step with `GeneratorError.Cancelled`.
    public func generate(
        pcm: [Float],
        sampleRate: Double,
        request: GenerateChartRequest,
        progress: @escaping @Sendable (GenerationPhase) -> Void = { _ in }
    ) async throws -> ChartResult {
        let cancellation = Cancellation()
        let listener = ProgressListener(progress)
        return try await withTaskCancellationHandler {
            try await onGenerationThread {
                try self.generate(pcm: pcm, sampleRate: sampleRate, request: request, listener: listener, cancellation: cancellation)
            }
        } onCancel: {
            cancellation.cancel()
        }
    }

    /// Runs the stages that read only the audio of mono PCM at `sampleRate` on a thread of its own, for a
    /// `ChartScheduler` to generate from; `bytes()` stores the result. Cancelling the task stops it at its next step
    /// with `GeneratorError.Cancelled`.
    public func prepare(pcm: [Float], sampleRate: Double) async throws -> Prepared {
        let cancellation = Cancellation()
        return try await withTaskCancellationHandler {
            try await onGenerationThread { try self.prepare(pcm: pcm, sampleRate: sampleRate, cancellation: cancellation) }
        } onCancel: {
            cancellation.cancel()
        }
    }
}

extension ChartScheduler {
    /// `work()` on a thread of its own: generates until play may start (`.ready`) or nothing is left to do (`.done`),
    /// waiting while play is far enough ahead. Cancelling the task stops the scheduler, which then returns `.done`.
    public func work() async throws -> SchedulerAction {
        try await withTaskCancellationHandler {
            try await onGenerationThread { try self.work() }
        } onCancel: {
            self.stop()
        }
    }
}

/// Runs `body` on a new thread with the stack of a main thread, which ONNX Runtime and the generator need, not the
/// 512 KiB of a secondary one.
private func onGenerationThread<T>(_ body: @escaping () throws -> T) async throws -> T {
    try await withCheckedThrowingContinuation { continuation in
        let thread = Thread {
            continuation.resume(with: Result { try body() })
        }
        thread.stackSize = 8 << 20
        thread.qualityOfService = .userInitiated
        thread.start()
    }
}

private final class ProgressListener: GenerationListener {
    private let report: @Sendable (GenerationPhase) -> Void

    init(_ report: @escaping @Sendable (GenerationPhase) -> Void) {
        self.report = report
    }

    func progress(phase: GenerationPhase) {
        report(phase)
    }
}
