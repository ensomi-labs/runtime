import Foundation

extension ChartGenerator {
    /// Generates a chart from mono PCM at `sampleRate` on a thread of its own. `progress` hears each phase on
    /// that thread. Cancelling the task stops the generation at its next step with `GeneratorError.Cancelled`.
    public func generate(
        pcm: [Float],
        sampleRate: Double,
        request: GenerateChartRequest,
        progress: @escaping @Sendable (GenerationPhase) -> Void = { _ in }
    ) async throws -> ChartResult {
        let bytes = pcm.withUnsafeBufferPointer { Data(buffer: $0) }
        let cancellation = Cancellation()
        let listener = ProgressListener(progress)
        return try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { continuation in
                let thread = Thread {
                    continuation.resume(with: Result {
                        try self.generate(
                            pcm: bytes, sampleRate: sampleRate, request: request, listener: listener, cancellation: cancellation
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
