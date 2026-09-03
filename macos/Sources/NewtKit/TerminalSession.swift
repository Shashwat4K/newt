import CNewt
import Foundation

/// A running terminal session: one PTY, one child process, one screen.
///
/// This is the only type that touches the raw ABI. Terminal semantics live in
/// the core — if a fix concerns what an escape sequence *means*, it does not
/// belong in this file, even when the symptom is visual.
public final class TerminalSession {
    private let handle: OpaquePointer

    /// The raw ABI handle, for the input extensions in this module.
    var rawHandle: OpaquePointer { handle }

    /// Start a session from a full spec: program, arguments, environment, cwd.
    ///
    /// This is the general constructor. Running an agent CLI needs it — an
    /// argument list cannot be expressed any other way.
    public init(spec: SessionSpec) throws {
        let created = spec.withNativeSpec { newt_session_open($0) }

        guard let created else {
            throw TerminalError.lastError(fallback: "could not start the terminal session")
        }
        handle = created
    }

    /// Start a session running a program with no arguments, or the login shell.
    ///
    /// The common case, kept as a convenience so a plain terminal tab does not
    /// have to build a spec to say "just give me a shell".
    ///
    /// - Parameters:
    ///   - size: initial grid size in cells.
    ///   - shell: program to run; nil uses the user's login shell.
    ///   - workingDirectory: nil uses the process working directory.
    ///   - scrollbackLines: lines of history to retain.
    public convenience init(
        size: TerminalSize,
        shell: String? = nil,
        workingDirectory: String? = nil,
        scrollbackLines: UInt32 = 10_000
    ) throws {
        try self.init(
            spec: SessionSpec(
                size: size,
                program: shell,
                workingDirectory: workingDirectory,
                scrollbackLines: scrollbackLines
            )
        )
    }

    deinit {
        newt_session_free(handle)
    }

    /// Send input to the child process.
    public func write(_ bytes: [UInt8]) throws {
        let ok = bytes.withUnsafeBufferPointer { buffer in
            newt_session_write(handle, buffer.baseAddress, UInt(buffer.count))
        }
        guard ok else { throw TerminalError.lastError(fallback: "write failed") }
    }

    /// Send input as UTF-8.
    public func write(_ text: String) throws {
        try write(Array(text.utf8))
    }

    /// Resize the grid and inform the child process.
    public func resize(to size: TerminalSize) throws {
        guard newt_session_resize(handle, size.cols, size.rows) else {
            throw TerminalError.lastError(fallback: "resize failed")
        }
    }

    /// Scroll the viewport; positive scrolls back into history.
    public func scroll(by lines: Int32) {
        _ = newt_session_scroll(handle, lines)
    }

    /// Report cell metrics so the terminal can answer pixel-size queries.
    public func setCellSize(width: UInt16, height: UInt16) {
        _ = newt_session_set_cell_size(handle, width, height)
    }

    /// Whether the child process has closed the terminal.
    public var hasExited: Bool {
        newt_session_has_exited(handle)
    }

    /// Window title set by the child, if any.
    public var title: String? {
        guard let pointer = newt_session_title(handle) else { return nil }
        return String(cString: pointer)
    }

    /// Read the current screen.
    ///
    /// The snapshot borrows session-owned memory and is only valid inside
    /// `body`; escaping it leaves dangling pointers.
    @discardableResult
    public func withSnapshot<R>(_ body: (GridSnapshot) throws -> R) throws -> R {
        var raw = NewtSnapshot()
        guard newt_session_snapshot(handle, &raw) else {
            throw TerminalError.lastError(fallback: "snapshot failed")
        }
        return try body(GridSnapshot(raw))
    }

    private static func withOptionalCString<R>(
        _ value: String?,
        _ body: (UnsafePointer<CChar>?) -> R
    ) -> R {
        guard let value else { return body(nil) }
        return value.withCString { body($0) }
    }
}
