import XCTest
@testable import NewtKit

/// The seam for the end goal: a session carries token, cost, and agent state as
/// first-class fields so those features can attach later without widening every
/// layer at once.
final class SessionMetadataTests: XCTestCase {
    private func makeSession() throws -> TerminalSession {
        try TerminalSession(size: TerminalSize(cols: 40, rows: 8), shell: "/bin/sh")
    }

    func testANewSessionReportsNothingRatherThanZero() throws {
        let session = try makeSession()
        let metadata = session.metadata

        // `unknown` is distinct from `idle`: no agent has reported at all.
        XCTAssertEqual(metadata.agentState, .unknown)
        XCTAssertNil(metadata.model)
        XCTAssertEqual(metadata.totalTokens, 0)
    }

    func testMetadataRoundTripsThroughTheABI() throws {
        let session = try makeSession()
        let written = SessionMetadata(
            inputTokens: 1_200,
            outputTokens: 340,
            costMicros: 15_750,
            agentState: .running,
            model: "claude-opus-5"
        )

        try session.updateMetadata(written)

        XCTAssertEqual(session.metadata, written)
        XCTAssertEqual(session.metadata.totalTokens, 1_540)
    }

    func testClearingTheModelIsDistinctFromAnEmptyName() throws {
        let session = try makeSession()
        try session.updateMetadata(SessionMetadata(model: "some-model"))
        XCTAssertEqual(session.metadata.model, "some-model")

        try session.updateMetadata(SessionMetadata(model: nil))

        XCTAssertNil(session.metadata.model)
    }

    /// Metadata is bookkeeping, not terminal state — updating it must not
    /// disturb anything that gets drawn.
    func testMetadataDoesNotAffectTheGrid() throws {
        let session = try makeSession()
        try session.write("printf 'grid content\\n'\n")

        let deadline = Date().addingTimeInterval(5)
        var appeared = false
        while Date() < deadline, !appeared {
            Thread.sleep(forTimeInterval: 0.02)
            appeared = try session.withSnapshot { $0.text().contains("grid content") }
        }
        XCTAssertTrue(appeared)

        let before = try session.withSnapshot { $0.text() }
        try session.updateMetadata(SessionMetadata(inputTokens: 99, agentState: .waiting))
        let after = try session.withSnapshot { $0.text() }

        XCTAssertEqual(before, after)
    }

    func testEachSessionKeepsItsOwnMetadata() throws {
        let first = try makeSession()
        let second = try makeSession()

        try first.updateMetadata(SessionMetadata(inputTokens: 10, agentState: .running))
        try second.updateMetadata(SessionMetadata(inputTokens: 20, agentState: .idle))

        XCTAssertEqual(first.metadata.inputTokens, 10)
        XCTAssertEqual(first.metadata.agentState, .running)
        XCTAssertEqual(second.metadata.inputTokens, 20)
        XCTAssertEqual(second.metadata.agentState, .idle)
    }
}
