import Foundation
import NewtKit

// Phase 2 stub: proves the shell can drive a real session through the C ABI and
// read the grid back. Phase 3 replaces this with an NSApplication, a window,
// and the CoreText renderer; the session code below stays as it is.

let size = TerminalSize(cols: 80, rows: 20)

do {
    let session = try TerminalSession(size: size)
    let command = CommandLine.arguments.dropFirst().first ?? "echo newt is alive"

    // Wait for the prompt before typing at it: a heavy shell startup can take
    // seconds to emit its first byte.
    try waitForOutput(session)
    try session.write("\(command)\n")
    try settle(session)

    try session.withSnapshot { snapshot in
        print(snapshot.text())
        FileHandle.standardError.write(Data("""

        [\(snapshot.cols)x\(snapshot.rows) \
        cursor \(snapshot.cursor.col),\(snapshot.cursor.row) \
        history \(snapshot.historyLength) \
        core \(Core.version)]

        """.utf8))
    }
} catch {
    FileHandle.standardError.write(Data("newt: \(error)\n".utf8))
    exit(1)
}

/// Block until the screen produces anything, so an empty grid is not mistaken
/// for a settled one.
func waitForOutput(_ session: TerminalSession, timeout: TimeInterval = 10) throws {
    let deadline = Date().addingTimeInterval(timeout)
    while Date() < deadline {
        Thread.sleep(forTimeInterval: 0.02)
        let hasOutput = try session.withSnapshot { !$0.text().isEmpty }
        if hasOutput { return }
    }
}

/// Block until the screen stops changing.
func settle(
    _ session: TerminalSession,
    quietPeriod: TimeInterval = 0.3,
    timeout: TimeInterval = 10
) throws {
    let deadline = Date().addingTimeInterval(timeout)
    var last = try session.withSnapshot { $0.text() }
    var unchangedSince = Date()

    while Date() < deadline {
        Thread.sleep(forTimeInterval: 0.02)
        let current = try session.withSnapshot { $0.text() }
        if current != last {
            last = current
            unchangedSince = Date()
        } else if Date().timeIntervalSince(unchangedSince) >= quietPeriod {
            return
        }
    }
}
