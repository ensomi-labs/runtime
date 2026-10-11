import Foundation
import XCTest
@testable import EnsomiRuntime

/// Runs every conformance/gameplay fixture through the Swift API and compares the output.
final class ConformanceTests: XCTestCase {
    func testEveryFixtureReproducesItsExpectedOutput() throws {
        let fixtures = try fixtureFiles("conformance/gameplay", pathExtension: "json")
        XCTAssertFalse(fixtures.isEmpty)
        for url in fixtures {
            try run(url)
        }
    }

    func testEveryOsuFileParsesAsExpected() throws {
        let files = try fixtureFiles("conformance/gameplay/osu", pathExtension: "osu")
        XCTAssertFalse(files.isEmpty)
        for url in files {
            try parse(url)
        }
    }

    func testStreamingIsReachable() throws {
        let session = GameplaySession.streaming(ruleset: Ruleset(judge: .c))
        XCTAssertTrue(try session.appendTokens(
            tokens: [ChartToken(lane: 0, timeMs: 1_000, kind: .holdStart)], coverageMs: 1_100, endOfStream: false
        ).isEmpty)
        XCTAssertEqual(session.visibleObjects(lowerMs: 500, upperMs: 1_500).first?.state, .openEnded)
        XCTAssertTrue(session.advance(watermarkMs: 2_000).isEmpty)
        XCTAssertEqual(session.decidedThroughMs(), 1_100)
        XCTAssertThrowsError(try session.appendTokens(tokens: [], coverageMs: 1_100, endOfStream: true)) { error in
            XCTAssertEqual(error as? ChartError, .UnclosedHold(object: 0))
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

    private func fixtureFiles(_ path: String, pathExtension: String) throws -> [URL] {
        // .../swift/Tests/EnsomiRuntimeTests/ConformanceTests.swift -> repository root
        let root = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent()
            .deletingLastPathComponent().deletingLastPathComponent()
        let directory = root.appendingPathComponent(path)
        return try FileManager.default.contentsOfDirectory(at: directory, includingPropertiesForKeys: nil)
            .filter { $0.pathExtension == pathExtension }
            .sorted { $0.lastPathComponent < $1.lastPathComponent }
    }

    private func object(_ object: ObjectFixture) -> ChartObject {
        ChartObject(lane: object.lane, startMs: object.startMs, endMs: object.endMs)
    }

    private func run(_ url: URL) throws {
        let name = url.lastPathComponent
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        let fixture = try decoder.decode(Fixture.self, from: Data(contentsOf: url))

        let judge: Judge = ["A": .a, "B": .b, "C": .c, "D": .d, "E": .e][fixture.ruleset.judge]!
        let session: GameplaySession
        if let chart = fixture.chart {
            session = try GameplaySession(
                chart: Chart(durationMs: chart.durationMs, objects: chart.objects.map(object)),
                ruleset: Ruleset(judge: judge)
            )
        } else {
            session = GameplaySession.streaming(ruleset: Ruleset(judge: judge))
        }

        let kinds: [String: TokenKind] = ["tap": .tap, "hold_start": .holdStart, "hold_end": .holdEnd]
        var events: [(event: JudgementEvent, step: Int)] = []
        for (index, step) in fixture.steps.enumerated() {
            var produced: [JudgementEvent] = []
            if let input = step.input {
                let phase: Phase = input.phase == "press" ? .press : .release
                produced = session.input(input: InputEvent(lane: input.lane, phase: phase, timeMs: input.timeMs))
            } else if let watermark = step.advance {
                produced = session.advance(watermarkMs: watermark)
            } else if let page = step.page {
                if let tokens = page.tokens {
                    produced = try session.appendTokens(
                        tokens: tokens.map { ChartToken(lane: $0.lane, timeMs: $0.timeMs, kind: kinds[$0.kind]!) },
                        coverageMs: page.coverageMs,
                        endOfStream: page.endOfStream
                    )
                } else {
                    produced = try session.append(
                        objects: (page.objects ?? []).map(object),
                        coverageMs: page.coverageMs,
                        endOfStream: page.endOfStream
                    )
                }
            } else if let decidedThrough = step.decidedThroughMs {
                XCTAssertEqual(session.decidedThroughMs(), decidedThrough, "\(name): step \(index) decided through")
            }
            events += produced.map { ($0, index) }
        }

        XCTAssertEqual(events.count, fixture.expected.events.count, "\(name): event count")
        for (index, ((actual, step), expected)) in zip(events, fixture.expected.events).enumerated() {
            let label = "\(name): event \(index)"
            if let expectedStep = expected.step {
                XCTAssertEqual(step, expectedStep, label)
            }
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
        XCTAssertTrue(session.isResolved(), "\(name): resolved")
    }

    private func parse(_ url: URL) throws {
        let name = url.lastPathComponent
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        let expected = try decoder.decode(
            OsuFixture.self, from: Data(contentsOf: url.deletingPathExtension().appendingPathExtension("json"))
        ).expected
        do {
            let beatmap = try parseOsu(text: String(contentsOf: url, encoding: .utf8))
            guard let chart = expected.chart else {
                return XCTFail("\(name): parsed, expected \(String(describing: expected.error))")
            }
            XCTAssertEqual(beatmap.chart, Chart(durationMs: chart.durationMs, objects: chart.objects.map(object)), name)
            XCTAssertEqual(beatmap.title, expected.title, name)
            XCTAssertEqual(beatmap.artist, expected.artist, name)
        } catch let error as OsuError {
            guard let expectedError = expected.error else {
                return XCTFail("\(name): \(error)")
            }
            let chartErrors: [String: (UInt32) -> ChartError] = [
                "same_lane_overlap": ChartError.SameLaneOverlap(object:),
                "not_sorted": ChartError.NotSorted(object:),
            ]
            switch (error, expectedError.kind) {
            case (.UnsupportedMode, "unsupported_mode"):
                break
            case (.UnsupportedKeyCount(let keys), "unsupported_key_count"):
                XCTAssertEqual(keys, expectedError.keys, name)
            case (.UnsupportedHitObject(let line), "unsupported_hit_object"):
                XCTAssertEqual(line, expectedError.line, name)
            case (.InvalidChart(let chartError), "invalid_chart"):
                XCTAssertEqual(chartError, chartErrors[expectedError.chartError ?? ""]?(expectedError.object ?? 0), name)
            default:
                XCTFail("\(name): \(error)")
            }
        }
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
    var chart: ChartFixture?
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
    var page: PageFixture?
    var decidedThroughMs: Double?
}

private struct PageFixture: Decodable {
    var objects: [ObjectFixture]?
    var tokens: [TokenFixture]?
    var coverageMs: Double
    var endOfStream: Bool
}

private struct TokenFixture: Decodable {
    var lane: UInt32
    var timeMs: Double
    var kind: String
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
    var step: Int?
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

private struct OsuFixture: Decodable {
    var expected: OsuExpected
}

private struct OsuExpected: Decodable {
    var chart: ChartFixture?
    var title: String?
    var artist: String?
    var error: OsuErrorFixture?
}

private struct OsuErrorFixture: Decodable {
    var kind: String
    var keys: Int64?
    var line: String?
    var chartError: String?
    var object: UInt32?
}
