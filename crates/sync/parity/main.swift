// Writes what `examples/parity.rs` compares, by the Swift app's own decoding and frame building. It is compiled together
// with the app's sources (README.md beside this file), which is why it names their types without importing anything.
//
//   export FIXTURE_DIRECTORY OUTPUT_DIRECTORY    every fixture's sidecar, recording and reference
//   export --conformance OUTPUT_DIRECTORY        the small fixture of conformance/sync
//
// Identifiers in the output are running numbers. No title or file name is copied.

import AVFoundation
import Foundation

let builder = AmbientSyncAudioFeatureFrameBuilder()

func write<T>(_ values: [T], to url: URL) throws {
    try values.withUnsafeBytes { try Data($0).write(to: url) }
}

func exportFrames(_ frames: [MicFeatureFrame], to directory: URL) throws {
    try write(frames.map(\.recordedTimeMS), to: directory.appendingPathComponent("times.f64"))
    try write(frames.map(\.energyDBFS), to: directory.appendingPathComponent("energy.f64"))
    try write(frames.flatMap(\.pcenMel), to: directory.appendingPathComponent("pcen.f32"))
}

/// The file's PCM as the app decodes it, mixed to mono, at the file's rate.
func sourcePCM(_ url: URL) throws -> (samples: [Float], rate: Double) {
    let file = try AVAudioFile(forReading: url, commonFormat: .pcmFormatFloat32, interleaved: false)
    let format = file.processingFormat
    guard let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: AVAudioFrameCount(file.length)) else {
        throw CocoaError(.fileReadUnknown)
    }
    try file.read(into: buffer)
    let channels = Int(format.channelCount)
    var mono = [Float](repeating: 0, count: Int(buffer.frameLength))
    for channel in 0..<channels {
        let source = buffer.floatChannelData![channel]
        for frame in mono.indices { mono[frame] += source[frame] }
    }
    if channels > 1 {
        for frame in mono.indices { mono[frame] /= Float(channels) }
    }
    return (mono, format.sampleRate)
}

func exportAudio(_ url: URL, to directory: URL, withSource: Bool) throws {
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    let audio = try builder.decodeMonoFloat32Audio(from: url)
    try write(audio.monoSamples, to: directory.appendingPathComponent("pcm.f32"))
    try exportFrames(builder.buildFrames(fromMonoSamples: audio.monoSamples, sampleRate: audio.sampleRate), to: directory)
    if withSource {
        let source = try sourcePCM(url)
        try write(source.samples, to: directory.appendingPathComponent("source.f32"))
        try "\(Int(source.rate))\n".write(to: directory.appendingPathComponent("source-rate"), atomically: true, encoding: .utf8)
    }
}

func exportFixtures(from fixtures: URL, to output: URL) throws {
    var references: [String: String] = [:]
    var cases: [(id: String, query: String, target: String)] = []
    let sidecars = try FileManager.default.contentsOfDirectory(at: fixtures, includingPropertiesForKeys: nil)
        .filter { $0.lastPathComponent.hasSuffix(".ambient-sync-fixture.json") }
        .sorted { $0.lastPathComponent < $1.lastPathComponent }
    for sidecar in sidecars {
        let metadata = try JSONSerialization.jsonObject(with: Data(contentsOf: sidecar)) as! [String: Any]
        let target = (metadata["targetAsset"] as! [String: Any])["displayPath"] as! String
        let recording = fixtures.appendingPathComponent(metadata["audioFileName"] as! String)
        guard FileManager.default.fileExists(atPath: target), FileManager.default.fileExists(atPath: recording.path) else {
            print("skipped a fixture with a missing file")
            continue
        }
        if references[target] == nil {
            let name = "references/\(String(format: "%02d", references.count))"
            try exportAudio(URL(fileURLWithPath: target), to: output.appendingPathComponent(name), withSource: true)
            references[target] = name
        }
        let id = String(format: "fixture-%02d", cases.count)
        try exportAudio(recording, to: output.appendingPathComponent("queries/\(id)"), withSource: false)
        cases.append((id, "queries/\(id)", target))
        print("exported \(id)")
    }
    // A query's wrong reference is the next song's.
    let names = references.values.sorted()
    let lines = cases.map { item -> String in
        let reference = references[item.target]!
        let wrong = names[(names.firstIndex(of: reference)! + 1) % names.count]
        return [item.id, item.query, reference, wrong].joined(separator: "\t")
    }
    try (lines.joined(separator: "\n") + "\n").write(to: output.appendingPathComponent("cases.tsv"), atomically: true, encoding: .utf8)
}

/// Sixteen frames of a deterministic signal that passes through silence, tones, a click and noise.
func exportConformance(to output: URL) throws {
    var state: UInt32 = 1
    var pcm = [Float](repeating: 0, count: 1_024 + 15 * 512)
    for index in pcm.indices {
        state = state &* 1_664_525 &+ 1_013_904_223
        let noise = Float(state >> 8) / Float(1 << 24) - 0.5
        let time = Double(index) / 48_000
        switch index {
        case ..<1_200: pcm[index] = 0
        case ..<4_000: pcm[index] = Float(0.4 * sin(2 * .pi * 440 * time) + 0.2 * sin(2 * .pi * 3_100 * time))
        case 4_000: pcm[index] = 0.9
        case ..<6_000: pcm[index] = 0.02 * noise
        default: pcm[index] = 0.6 * noise + Float(0.1 * sin(2 * .pi * 90 * time))
        }
    }
    try write(pcm, to: output.appendingPathComponent("frames.f32"))
    let frames = builder.buildFrames(fromMonoSamples: pcm, sampleRate: 48_000)
    let json: [String: Any] = [
        "sampleRate": 48_000,
        "frames": frames.map { frame -> [String: Any] in
            ["recordedTimeMS": frame.recordedTimeMS, "energyDBFS": frame.energyDBFS, "pcenMel": frame.pcenMel.map(Double.init)]
        },
    ]
    let data = try JSONSerialization.data(withJSONObject: json, options: [.prettyPrinted, .sortedKeys])
    try data.write(to: output.appendingPathComponent("frames.json"))
}

let arguments = CommandLine.arguments
if arguments.count == 3, arguments[1] == "--conformance" {
    try exportConformance(to: URL(fileURLWithPath: arguments[2]))
} else if arguments.count == 3 {
    try exportFixtures(from: URL(fileURLWithPath: arguments[1]), to: URL(fileURLWithPath: arguments[2]))
} else {
    print("usage: export FIXTURE_DIRECTORY OUTPUT_DIRECTORY | export --conformance OUTPUT_DIRECTORY")
    exit(2)
}
