import CNewt
import XCTest
@testable import NewtKit

/// The encoding itself is tested exhaustively in the core; these cover the
/// plumbing — that events cross the ABI and reach the child intact.
final class InputTests: XCTestCase {
    private func makeSession() throws -> TerminalSession {
        try TerminalSession(size: TerminalSize(cols: 60, rows: 10), shell: "/bin/sh")
    }

    private func poll(
        _ session: TerminalSession,
        for needle: String,
        timeout: TimeInterval = 5
    ) throws -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            Thread.sleep(forTimeInterval: 0.02)
            if try session.withSnapshot({ $0.text().contains(needle) }) { return true }
        }
        return false
    }

    func testTypedKeysReachTheShell() throws {
        let session = try makeSession()

        for scalar in "printf 'typed'".unicodeScalars {
            try session.send(key: .character(scalar))
        }
        try session.send(key: .enter)

        XCTAssertTrue(try poll(session, for: "typed"), "typed keys never reached the shell")
    }

    func testPasteIsDelivered() throws {
        let session = try makeSession()
        // The core turns the newline into a carriage return, which the shell
        // sees as Return and runs the line.
        try session.paste("printf 'pasted'\n")

        XCTAssertTrue(try poll(session, for: "pasted"), "pasted text never reached the shell")
    }

    func testControlKeysAreAccepted() throws {
        let session = try makeSession()
        for scalar in "sleep 30".unicodeScalars {
            try session.send(key: .character(scalar))
        }
        try session.send(key: .enter)
        Thread.sleep(forTimeInterval: 0.2)

        // Control-C should interrupt it and return a prompt.
        try session.send(key: .character("c"), modifiers: .control)
        try session.send(key: .enter)
        for scalar in "printf 'after'".unicodeScalars {
            try session.send(key: .character(scalar))
        }
        try session.send(key: .enter)

        XCTAssertTrue(
            try poll(session, for: "after"),
            "the shell never became responsive again after Control-C"
        )
    }

    /// A plain `sh` enables no mouse reporting, so nothing should be sent — the
    /// caller needs that answer to fall back to scrolling its own viewport.
    func testMouseIsNotSentWhenTheProgramIsNotAskingForIt() throws {
        let session = try makeSession()
        let handled = try session.send(mouse: .press, button: .left, col: 3, row: 4)
        XCTAssertFalse(handled)

        let scrolled = try session.send(mouse: .scrollUp, col: 0, row: 0)
        XCTAssertFalse(scrolled)
    }

    func testCommandModifierSendsNothing() throws {
        let session = try makeSession()
        // Command chords are application shortcuts; the child must not see them.
        try session.send(key: .character("c"), modifiers: .command)
        try session.send(key: .character("q"), modifiers: [.command, .shift])
        // Nothing to assert beyond not throwing and not corrupting the session.
        for scalar in "printf 'still-alive'".unicodeScalars {
            try session.send(key: .character(scalar))
        }
        try session.send(key: .enter)

        XCTAssertTrue(try poll(session, for: "still-alive"))
    }

    func testKeyIdentifiersMatchTheABIConstants() {
        XCTAssertEqual(TerminalKey.enter.identifier, UInt32(NEWT_KEY_ENTER))
        XCTAssertEqual(TerminalKey.pageDown.identifier, UInt32(NEWT_KEY_PAGE_DOWN))
        XCTAssertEqual(TerminalKey.function(1).identifier, UInt32(NEWT_KEY_F1))
        XCTAssertEqual(TerminalKey.function(12).identifier, UInt32(NEWT_KEY_F1) + 11)
        XCTAssertEqual(TerminalKey.character("a").identifier, 97)
    }
}
