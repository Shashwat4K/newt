import CNewt
import XCTest

@testable import NewtKit

/// The spec crosses the ABI as pointers into a buffer this side owns, so the
/// interesting failures are lifetime and offset bugs, not logic ones: a slice
/// pointing at freed storage, or at the right length in the wrong place. These
/// assert the encoding directly, then prove it end to end against a real child.
final class SessionSpecTests: XCTestCase {
    func testEveryFieldSurvivesEncoding() {
        let spec = SessionSpec(
            size: TerminalSize(cols: 80, rows: 24),
            program: "/bin/sh",
            arguments: ["-c", "echo hi"],
            environment: [("A", "1"), ("LONGER_NAME", "a longer value")],
            workingDirectory: "/tmp",
            term: "xterm-256color",
            scrollbackLines: 500
        )

        spec.withNativeSpec { native in
            XCTAssertEqual(native.pointee.cols, 80)
            XCTAssertEqual(native.pointee.rows, 24)
            XCTAssertEqual(native.pointee.scrollback_lines, 500)
            XCTAssertEqual(text(native.pointee.program), "/bin/sh")
            XCTAssertEqual(text(native.pointee.cwd), "/tmp")
            XCTAssertEqual(text(native.pointee.term), "xterm-256color")

            XCTAssertEqual(native.pointee.arg_count, 2)
            XCTAssertEqual(text(native.pointee.args![0]), "-c")
            XCTAssertEqual(text(native.pointee.args![1]), "echo hi")

            XCTAssertEqual(native.pointee.env_count, 2)
            XCTAssertEqual(text(native.pointee.env![0].key), "A")
            XCTAssertEqual(text(native.pointee.env![0].value), "1")
            XCTAssertEqual(text(native.pointee.env![1].key), "LONGER_NAME")
            XCTAssertEqual(text(native.pointee.env![1].value), "a longer value")
        }
    }

    func testOmittedFieldsBecomeEmptySlicesRatherThanDanglingOnes() {
        let spec = SessionSpec(size: TerminalSize(cols: 40, rows: 10))

        spec.withNativeSpec { native in
            // Empty must be a null slice, not a pointer to zero bytes at the
            // end of the blob: the core reads empty as "not supplied", and a
            // non-null pointer with a zero length invites someone to read it.
            XCTAssertNil(native.pointee.program.ptr)
            XCTAssertEqual(native.pointee.program.len, 0)
            XCTAssertNil(native.pointee.cwd.ptr)
            XCTAssertNil(native.pointee.term.ptr)
            XCTAssertEqual(native.pointee.arg_count, 0)
            XCTAssertEqual(native.pointee.env_count, 0)
        }
    }

    /// An empty argument is still an argument. `sh -c '' x` is not `sh -c x`.
    func testAnEmptyArgumentKeepsItsPositionAmongTheOthers() {
        let spec = SessionSpec(
            size: TerminalSize(cols: 40, rows: 10),
            program: "/bin/sh",
            arguments: ["first", "", "third"]
        )

        spec.withNativeSpec { native in
            XCTAssertEqual(native.pointee.arg_count, 3)
            XCTAssertEqual(text(native.pointee.args![0]), "first")
            XCTAssertEqual(text(native.pointee.args![1]), "")
            XCTAssertEqual(text(native.pointee.args![2]), "third")
        }
    }

    func testMultiByteTextCrossesAsBytesNotCharacters() {
        // The boundary counts bytes; a length in characters would truncate
        // mid-sequence and hand the core invalid UTF-8, which it rejects.
        let spec = SessionSpec(
            size: TerminalSize(cols: 40, rows: 10),
            program: "/bin/echo",
            arguments: ["héllo → 世界"],
            environment: [("EMOJI", "🧪")]
        )

        spec.withNativeSpec { native in
            XCTAssertEqual(text(native.pointee.args![0]), "héllo → 世界")
            XCTAssertEqual(native.pointee.args![0].len, UInt("héllo → 世界".utf8.count))
            XCTAssertEqual(text(native.pointee.env![0].value), "🧪")
        }
    }

    func testArgumentsAndEnvironmentReachTheChild() throws {
        let session = try TerminalSession(
            spec: SessionSpec(
                size: TerminalSize(cols: 40, rows: 8),
                program: "/bin/sh",
                arguments: ["-c", "printf %s \"$NEWT_SWIFT_VAR\""],
                environment: [("NEWT_SWIFT_VAR", "arrived")]
            )
        )

        let deadline = Date().addingTimeInterval(5)
        var found = false
        while Date() < deadline, !found {
            Thread.sleep(forTimeInterval: 0.02)
            found = try session.withSnapshot { $0.text().contains("arrived") }
        }
        XCTAssertTrue(found, "argv or env did not survive the Swift encoding")
    }

    func testTheConvenienceInitStillStartsAShell() throws {
        // The narrow constructor is now built on the spec; this is what proves
        // the two paths agree, since every other test in the suite uses it.
        let session = try TerminalSession(size: TerminalSize(cols: 40, rows: 8), shell: "/bin/sh")
        XCTAssertFalse(session.hasExited)
    }

    func testABadProgramFailsRatherThanReturningADeadSession() {
        XCTAssertThrowsError(
            try TerminalSession(
                spec: SessionSpec(
                    size: TerminalSize(cols: 40, rows: 8),
                    program: "/nonexistent/newt-should-not-find-this"
                )
            )
        )
    }

    private func text(_ bytes: NewtBytes) -> String {
        guard let pointer = bytes.ptr, bytes.len > 0 else { return "" }
        return String(decoding: UnsafeBufferPointer(start: pointer, count: Int(bytes.len)), as: UTF8.self)
    }
}
