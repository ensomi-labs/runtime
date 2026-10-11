import Foundation
import XCTest
@testable import EnsomiRuntime

/// Runs every conformance/gameplay fixture through the Swift API and compares the output.
final class ConformanceTests: XCTestCase {
    func testEveryFixtureReproducesItsExpectedOutput() throws {
        let fixtures = try fixtureFiles()
        XCTAssertFalse(fixtures.isEmpty)
        for url in fixtures {
            try run(url)
        }
    }

    func testFeedbackAndLayoutAreReachable() throws {
        let session = try GameplaySession(
            chart: Chart(durationMs: 3_000, objects: [ChartObject(lane: 0, startMs: 1_000, endMs: nil)]),
            ruleset: Ruleset(judge: .c)
        )
        let events = session.advance(watermarkMs: 1_500)
        XCTAssertEqual(events.map(\.tier), [.m])
        XCTAssertEqual(events.first?.judgement, .miss)
        XCTAssertEqual(session.score().tierCounts.missCount, 1)
        XCTAssertTrue(session.isResolved())

        let feedback = GameplayFeedback()
        feedback.recordJudgementEvents(events: events, chartTimeMs: 1_500)
        XCTAssertEqual(feedback.judgementPresentation(chartTimeMs: 1_500)?.event.judgement, .miss)
        XCTAssertEqual(feedback.judgementEventBatches().count, 1)

        let object = session.visibleObjects(lowerMs: 0, upperMs: 3_000)[0]
        XCTAssertEqual(object.state, .missedButVisible)
        guard case .tap(let y) = noteGeometry(object: object, renderTimeMs: 1_000, scrollTimeMs: 1_000, receptorY: 520) else {
            return XCTFail("expected tap geometry")
        }
        XCTAssertEqual(y, 520, accuracy: 0.001)
    }

    func testInvalidChartIsRejected() {
        XCTAssertThrowsError(try GameplaySession(
            chart: Chart(durationMs: 100, objects: [ChartObject(lane: 4, startMs: 10, endMs: nil)]),
            ruleset: Ruleset(judge: .c)
        )) { error in
            XCTAssertEqual(error as? ChartError, .InvalidLane(object: 0))
        }
    }

    // MARK: Fixtures

    private func fixtureFiles() throws -> [URL] {
        // .../swift/Tests/EnsomiRuntimeTests/ConformanceTests.swift -> repository root
        let root = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent()
            .deletingLastPathComponent().deletingLastPathComponent()
        let directory = root.appendingPathComponent("conformance/gameplay")
        return try FileManager.default.contentsOfDirectory(at: directory, includingPropertiesForKeys: nil)
            .filter { $0.pathExtension == "json" }
            .sorted { $0.lastPathComponent < $1.lastPathComponent }
    }

    private func run(_ url: URL) throws {
        let name = url.lastPathComponent
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        let fixture = try decoder.decode(Fixture.self, from: Data(contentsOf: url))

        let chart = Chart(
            durationMs: fixture.chart.durationMs,
            objects: fixture.chart.objects.map { ChartObject(lane: $0.lane, startMs: $0.startMs, endMs: $0.endMs) }
        )
        let judge: Judge = ["A": .a, "B": .b, "C": .c, "D": .d, "E": .e][fixture.ruleset.judge]!
        let session = try GameplaySession(chart: chart, ruleset: Ruleset(judge: judge))

        var events: [JudgementEvent] = []
        for step in fixture.steps {
            if let input = step.input {
                let phase: Phase = input.phase == "press" ? .press : .release
                events += session.input(input: InputEvent(lane: input.lane, phase: phase, timeMs: input.timeMs))
            } else if let watermark = step.advance {
                events += session.advance(watermarkMs: watermark)
            }
        }

        XCTAssertEqual(events.count, fixture.expected.events.count, "\(name): event count")
        for (index, (actual, expected)) in zip(events, fixture.expected.events).enumerated() {
            let label = "\(name): event \(index)"
            XCTAssertEqual(actual.object, expected.object, label)
            XCTAssertEqual(actual.lane, expected.lane, label)
            XCTAssertEqual(actual.part == .head ? "head" : "tail", expected.part, label)
            XCTAssertEqual(tierName(actual.tier), expected.tier, label)
            XCTAssertEqual(actual.timeMs, expected.timeMs, accuracy: 1e-6, label)
            XCTAssertEqual(actual.offsetMs == nil, expected.offsetMs == nil, label)
            if let actualOffset = actual.offsetMs, let expectedOffset = expected.offsetMs {
                XCTAssertEqual(actualOffset, expectedOffset, accuracy: 1e-6, label)
            }
        }

        let score = session.score()
        let counts = fixture.expected.score.tierCounts
        XCTAssertEqual(
            [score.tierCounts.bigP, score.tierCounts.p1, score.tierCounts.p2, score.tierCounts.p3, score.tierCounts.g, score.tierCounts.m],
            [counts.bigP, counts.p1, counts.p2, counts.p3, counts.g, counts.m],
            "\(name): tier counts"
        )
        XCTAssertEqual(score.accuracy, fixture.expected.score.accuracy, accuracy: 1e-9, "\(name): accuracy")
        XCTAssertEqual(score.combo, fixture.expected.score.combo, "\(name): combo")
        XCTAssertEqual(score.maxCombo, fixture.expected.score.maxCombo, "\(name): max combo")
        XCTAssertEqual(
            score.suggestedAudioOffsetMs == nil,
            fixture.expected.score.suggestedAudioOffsetMs == nil,
            "\(name): suggested offset presence"
        )
        if let actualOffset = score.suggestedAudioOffsetMs, let expectedOffset = fixture.expected.score.suggestedAudioOffsetMs {
            XCTAssertEqual(actualOffset, expectedOffset, accuracy: 1e-9, "\(name): suggested offset")
        }
        XCTAssertEqual(session.lateInputs(), fixture.expected.lateInputs, "\(name): late inputs")
    }

    private func tierName(_ tier: Tier) -> String {
        switch tier {
        case .bigP: "bigP"
        case .p1: "p1"
        case .p2: "p2"
        case .p3: "p3"
        case .g: "g"
        case .m: "m"
        }
    }
}

private struct Fixture: Decodable {
    var chart: ChartFixture
    var ruleset: RulesetFixture
    var steps: [StepFixture]
    var expected: ExpectedFixture
}

private struct ChartFixture: Decodable {
    var durationMs: UInt32
    var objects: [ObjectFixture]
}

private struct ObjectFixture: Decodable {
    var lane: UInt32
    var startMs: UInt32
    var endMs: UInt32?
}

private struct RulesetFixture: Decodable {
    var judge: String
}

private struct StepFixture: Decodable {
    var input: InputFixture?
    var advance: Double?
}

private struct InputFixture: Decodable {
    var lane: UInt32
    var phase: String
    var timeMs: Double
}

private struct ExpectedFixture: Decodable {
    var events: [EventFixture]
    var score: ScoreFixture
    var lateInputs: UInt32
}

private struct EventFixture: Decodable {
    var object: UInt32
    var lane: UInt32
    var part: String
    var tier: String
    var offsetMs: Double?
    var timeMs: Double
}

private struct ScoreFixture: Decodable {
    var tierCounts: TierCountsFixture
    var accuracy: Double
    var combo: UInt32
    var maxCombo: UInt32
    var suggestedAudioOffsetMs: Double?
}

private struct TierCountsFixture: Decodable {
    var bigP: UInt32
    var p1: UInt32
    var p2: UInt32
    var p3: UInt32
    var g: UInt32
    var m: UInt32
}
