import CNewt
import XCTest
@testable import NewtKit

final class CoreTests: XCTestCase {
    func testCoreVersionCrossesTheABIIntact() {
        let version = Core.version
        XCTAssertFalse(version.isEmpty)
        XCTAssertTrue(version.contains("."))
    }

    func testInvalidSizeIsReportedAsAnError() {
        XCTAssertThrowsError(try TerminalSession(size: TerminalSize(cols: 0, rows: 24))) { error in
            let message = String(describing: error)
            XCTAssertTrue(message.contains("size"), "unexpected message: \(message)")
        }
    }

    /// The whole path a shell exercises every frame: start a session, send
    /// input, and read the grid back through the ABI.
    func testSessionRoundTrip() throws {
        let session = try TerminalSession(
            size: TerminalSize(cols: 40, rows: 8),
            shell: "/bin/sh"
        )
        try session.write("printf 'abc'\n")

        let found = try pollForOutput(session, containing: "abc")
        XCTAssertTrue(found, "output never reached the snapshot")

        try session.withSnapshot { snapshot in
            XCTAssertEqual(snapshot.cols, 40)
            XCTAssertEqual(snapshot.rows, 8)
            XCTAssertEqual(snapshot.cells.count, 320)
        }
    }

    func testResizeIsReflectedInTheSnapshot() throws {
        let session = try TerminalSession(
            size: TerminalSize(cols: 40, rows: 8),
            shell: "/bin/sh"
        )
        try session.resize(to: TerminalSize(cols: 60, rows: 12))

        try session.withSnapshot { snapshot in
            XCTAssertEqual(snapshot.cols, 60)
            XCTAssertEqual(snapshot.rows, 12)
        }
    }

    /// Combining marks live in a side table rather than in the cell, so
    /// reassembling them is the shell's job and worth pinning down.
    func testCombiningMarksReassembleIntoOneCharacter() throws {
        let session = try TerminalSession(
            size: TerminalSize(cols: 20, rows: 4),
            shell: "/bin/sh"
        )
        // 'e' followed by U+0301 COMBINING ACUTE ACCENT.
        try session.write("printf 'e\\xcc\\x81'\n")

        let found = try pollForOutput(session, containing: "é")
        XCTAssertTrue(found, "combining mark was not reassembled")
    }

    func testWideSpacersAreNotDrawn() throws {
        let session = try TerminalSession(
            size: TerminalSize(cols: 20, rows: 4),
            shell: "/bin/sh"
        )
        try session.write("printf '中'\n")

        let found = try pollForOutput(session, containing: "中")
        XCTAssertTrue(found, "wide glyph did not render")

        try session.withSnapshot { snapshot in
            // Find the wide glyph and confirm its spacer carries no character.
            for row in 0..<snapshot.rows {
                for col in 0..<snapshot.cols {
                    guard let cell = snapshot.cell(col: col, row: row),
                          cell.flags & UInt16(NEWT_FLAG_WIDE) != 0 else { continue }
                    let spacer = snapshot.cell(col: col + 1, row: row)
                    XCTAssertNotNil(spacer)
                    XCTAssertNotEqual(spacer!.flags & UInt16(NEWT_FLAG_WIDE_SPACER), 0)
                    XCTAssertNil(snapshot.character(at: spacer!))
                    return
                }
            }
            XCTFail("no wide cell was found")
        }
    }

    /// Poll rather than sleep a fixed interval: shell startup time varies far
    /// more than the output does.
    private func pollForOutput(
        _ session: TerminalSession,
        containing needle: String,
        timeout: TimeInterval = 5
    ) throws -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            Thread.sleep(forTimeInterval: 0.02)
            let found = try session.withSnapshot { $0.text().contains(needle) }
            if found { return true }
        }
        return false
    }
}
