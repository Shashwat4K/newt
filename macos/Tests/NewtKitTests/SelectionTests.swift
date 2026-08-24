import CNewt
import XCTest
@testable import NewtKit

final class SelectionTests: XCTestCase {
    private func sessionWithText(_ text: String) throws -> TerminalSession {
        let session = try TerminalSession(
            size: TerminalSize(cols: 40, rows: 8),
            shell: "/bin/sh"
        )
        // The trailing newline matters: without it the shell prompt lands on
        // the same row as the output and no row is exactly the text.
        try session.write("printf '\(text)\\n'\n")

        let deadline = Date().addingTimeInterval(5)
        while Date() < deadline {
            Thread.sleep(forTimeInterval: 0.02)
            if try session.withSnapshot({ $0.text().contains(text) }) { return session }
        }
        XCTFail("output never appeared")
        return session
    }

    func testDraggedSelectionYieldsItsText() throws {
        let session = try sessionWithText("hello world")
        let row = try rowContaining("hello world", in: session)

        try session.startSelection(col: 0, row: row, sideRight: false, mode: .simple)
        try session.updateSelection(col: 4, row: row, sideRight: true)

        XCTAssertEqual(session.selectedText, "hello")
    }

    func testWordSelectionExpandsToTheWord() throws {
        let session = try sessionWithText("alpha beta")
        let row = try rowContaining("alpha beta", in: session)

        try session.startSelection(col: 7, row: row, sideRight: false, mode: .word)

        XCTAssertEqual(session.selectedText, "beta")
    }

    func testClearingRemovesTheSelection() throws {
        let session = try sessionWithText("something")
        let row = try rowContaining("something", in: session)

        try session.startSelection(col: 0, row: row, sideRight: false, mode: .simple)
        try session.updateSelection(col: 3, row: row, sideRight: true)
        XCTAssertNotNil(session.selectedText)

        session.clearSelection()

        XCTAssertNil(session.selectedText)
    }

    /// Selection reaches the renderer as a per-cell flag, so the drawing code
    /// never has to reason about wrapped lines.
    func testSelectedCellsAreFlaggedForTheRenderer() throws {
        let session = try sessionWithText("highlight me")
        let row = try rowContaining("highlight me", in: session)

        try session.startSelection(col: 0, row: row, sideRight: false, mode: .simple)
        try session.updateSelection(col: 8, row: row, sideRight: true)

        try session.withSnapshot { snapshot in
            let selected = (0...8).allSatisfy { col in
                (snapshot.cell(col: col, row: Int(row))?.flags ?? 0)
                    & UInt16(NEWT_FLAG_SELECTED) != 0
            }
            XCTAssertTrue(selected, "selected cells were not flagged")
        }
    }

    func testFindSelectsAMatch() throws {
        let session = try sessionWithText("alpha beta gamma")

        XCTAssertTrue(try session.find("beta"))
        XCTAssertEqual(session.selectedText, "beta")
    }

    func testFindReportsFailureForAbsentText() throws {
        let session = try sessionWithText("alpha beta")

        XCTAssertFalse(try session.find("zeta"))
    }

    /// Searching is literal: `a.out` should not match `about`.
    func testFindIsLiteralNotRegex() throws {
        let session = try sessionWithText("about")

        XCTAssertFalse(try session.find("a.out"))
    }

    func testEmptyQueryFindsNothing() throws {
        let session = try sessionWithText("anything")

        XCTAssertFalse(try session.find(""))
    }

    /// The row holding the command's *output*.
    ///
    /// Matching on "contains" would find the echoed command line first — it
    /// contains the same text inside the `printf` — and select from the wrong
    /// column.
    private func rowContaining(_ needle: String, in session: TerminalSession) throws -> UInt16 {
        try session.withSnapshot { snapshot in
            for row in 0..<snapshot.rows where snapshot.text(row: row) == needle {
                return UInt16(row)
            }
            XCTFail("no row was exactly \(needle)")
            return 0
        }
    }
}
