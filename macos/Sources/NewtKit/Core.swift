import CNewt

/// Entry point to the Rust core.
public enum Core {
    /// Version reported by the linked core library.
    public static var version: String {
        // Static storage owned by the core; valid for the process lifetime and
        // must not be freed, so a copying initializer is the correct read.
        String(cString: newt_version())
    }
}

/// A failure reported by the core.
public struct TerminalError: Error, CustomStringConvertible {
    public let message: String

    public var description: String { message }

    /// Read the core's last error for this thread, with a fallback so a failed
    /// call never surfaces as an empty message.
    static func lastError(fallback: String) -> TerminalError {
        guard let pointer = newt_last_error() else {
            return TerminalError(message: fallback)
        }
        return TerminalError(message: String(cString: pointer))
    }
}

/// Terminal dimensions in cells.
public struct TerminalSize: Equatable, Sendable {
    public var cols: UInt16
    public var rows: UInt16

    public init(cols: UInt16, rows: UInt16) {
        self.cols = cols
        self.rows = rows
    }
}
