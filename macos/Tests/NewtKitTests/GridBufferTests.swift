import CNewt
import XCTest
@testable import NewtKit

/// `GridBuffer` decides what the renderer repaints, so its damage bookkeeping
/// is worth pinning down: too little and the screen goes stale, too much and
/// every frame is a full redraw.
final class GridBufferTests: XCTestCase {
    private func makeSession() throws -> TerminalSession {
        try TerminalSession(size: TerminalSize(cols: 40, rows: 10), shell: "/bin/sh")
    }

    func testFirstUpdateDamagesEveryRow() throws {
        let session = try makeSession()
        var buffer = GridBuffer()

        try session.withSnapshot { buffer.update(from: $0) }

        XCTAssertEqual(buffer.cols, 40)
        XCTAssertEqual(buffer.rows, 10)
        XCTAssertEqual(buffer.damagedRows, Array(0..<10), "a first frame must repaint everything")
    }

    func testIdleUpdateDamagesOnlyTheCursorRows() throws {
        let session = try makeSession()
        var buffer = GridBuffer()

        try session.withSnapshot { buffer.update(from: $0) }
        // Let the shell settle so the second update sees a quiet terminal.
        Thread.sleep(forTimeInterval: 0.4)
        try session.withSnapshot { buffer.update(from: $0) }
        let cursorRowBefore = Int(buffer.cursor.row)
        try session.withSnapshot { buffer.update(from: $0) }

        let allowed = Set([cursorRowBefore, Int(buffer.cursor.row)])
        XCTAssertTrue(
            Set(buffer.damagedRows).isSubset(of: allowed),
            "idle frame repainted \(buffer.damagedRows), expected a subset of \(allowed)"
        )
    }

    func testOutputDamagesTheRowsItTouches() throws {
        let session = try makeSession()
        var buffer = GridBuffer()
        try session.withSnapshot { buffer.update(from: $0) }

        try session.write("printf 'hello'\n")
        var sawDamage = false
        let deadline = Date().addingTimeInterval(5)
        while Date() < deadline {
            Thread.sleep(forTimeInterval: 0.02)
            try session.withSnapshot { buffer.update(from: $0) }
            if buffer.text().contains("hello") {
                sawDamage = !buffer.damagedRows.isEmpty
                break
            }
        }

        XCTAssertTrue(sawDamage, "output arrived without any row being marked damaged")
    }

    func testBufferAgreesWithTheSnapshotItCopied() throws {
        let session = try makeSession()
        var buffer = GridBuffer()

        try session.write("printf 'agreement'\n")
        let deadline = Date().addingTimeInterval(5)
        var matched = false
        while Date() < deadline, !matched {
            Thread.sleep(forTimeInterval: 0.02)
            try session.withSnapshot { snapshot in
                buffer.update(from: snapshot)
                if snapshot.text().contains("agreement") {
                    XCTAssertEqual(buffer.text(row: 0), snapshot.text(row: 0))
                    matched = true
                }
            }
        }
        XCTAssertTrue(matched, "expected output never appeared")
    }

    func testCombiningMarksSurviveTheCopy() throws {
        let session = try makeSession()
        var buffer = GridBuffer()

        // 'e' followed by U+0301 COMBINING ACUTE ACCENT.
        try session.write("printf 'e\\xcc\\x81'\n")
        let deadline = Date().addingTimeInterval(5)
        var found = false
        while Date() < deadline, !found {
            Thread.sleep(forTimeInterval: 0.02)
            try session.withSnapshot { buffer.update(from: $0) }
            found = buffer.text().contains("é")
        }

        XCTAssertTrue(found, "combining mark did not survive the copy into the render buffer")
    }
}
