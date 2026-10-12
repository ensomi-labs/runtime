import Foundation

extension ChartGenerator {
    /// The resampler tables the package carries: `resample-<rate>.json`, `.bin` and `-boundary.bin`. Xcode copies
    /// the package's resource bundle into the app or test bundle that links the package, where `Bundle.module`, which
    /// aborts when it finds nothing, does not look if a framework links the package; so the loaded bundles come first.
    static let bundledResamplerDirectory: String? = {
        let copies = (Bundle.allBundles + Bundle.allFrameworks).lazy.compactMap { bundle in
            bundle.resourceURL.flatMap { Bundle(url: $0.appendingPathComponent("EnsomiRuntime_EnsomiRuntime.bundle")) }
        }
        return (copies.first ?? Bundle.module).url(forResource: "resampler", withExtension: nil)?.path
    }()

    /// Opens a model directory: `runtime.json` and the files it names, or the runtime config an EnsomiProtocol
    /// `ensomi-model.json` names. Without `options.resamplerDirectory`, PCM at a rate other than 22050 Hz is
    /// resampled with the tables the package carries.
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
            try await withCheckedThrowingContinuation { continuation in
                let thread = Thread {
                    continuation.resume(with: Result {
                        try self.generate(
                            pcm: pcm, sampleRate: sampleRate, request: request, listener: listener, cancellation: cancellation
                        )
                    })
                }
                // ONNX Runtime and the generator get the stack of a main thread, not the 512 KiB of a secondary one.
                thread.stackSize = 8 << 20
                thread.qualityOfService = .userInitiated
                thread.start()
            }
        } onCancel: {
            cancellation.cancel()
        }
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
