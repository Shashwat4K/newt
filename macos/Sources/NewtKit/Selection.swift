import CNewt

/// How a selection expands as it is dragged.
public enum SelectionMode: Sendable {
    /// Exactly the cells dragged over.
    case simple
    /// A rectangular region.
    case block
    /// Whole words, as a double-click gives.
    case word
    /// Whole lines, as a triple-click gives.
    case line

    var identifier: UInt8 {
        switch self {
        case .simple: return UInt8(NEWT_SELECTION_SIMPLE)
        case .block: return UInt8(NEWT_SELECTION_BLOCK)
        case .word: return UInt8(NEWT_SELECTION_WORD)
        case .line: return UInt8(NEWT_SELECTION_LINE)
        }
    }
}

extension TerminalSession {
    /// Begin a selection.
    ///
    /// - Parameter sideRight: which half of the cell the pointer is in, which
    ///   decides whether that cell is included.
    public func startSelection(
        col: UInt16,
        row: UInt16,
        sideRight: Bool,
        mode: SelectionMode
    ) throws {
        guard newt_session_selection_start(rawHandle, col, row, sideRight, mode.identifier) else {
            throw TerminalError.lastError(fallback: "starting a selection failed")
        }
    }

    public func updateSelection(col: UInt16, row: UInt16, sideRight: Bool) throws {
        guard newt_session_selection_update(rawHandle, col, row, sideRight) else {
            throw TerminalError.lastError(fallback: "updating the selection failed")
        }
    }

    public func clearSelection() {
        _ = newt_session_selection_clear(rawHandle)
    }

    /// The selected text, or nil when nothing is selected.
    public var selectedText: String? {
        guard let pointer = newt_session_selected_text(rawHandle) else { return nil }
        return String(cString: pointer)
    }

    /// Find `pattern`, selecting and scrolling to the match.
    ///
    /// The search is literal, not a regular expression.
    ///
    /// - Returns: whether a match was found.
    @discardableResult
    public func find(_ pattern: String, forward: Bool = true) throws -> Bool {
        guard !pattern.isEmpty else { return false }
        var found = false
        var utf8 = Array(pattern.utf8)
        let ok = utf8.withUnsafeMutableBufferPointer { buffer in
            newt_session_find(rawHandle, buffer.baseAddress, UInt(buffer.count), forward, &found)
        }
        guard ok else { throw TerminalError.lastError(fallback: "search failed") }
        return found
    }

    /// Jump the viewport back to the live edge.
    public func scrollToBottom() {
        _ = newt_session_scroll_to_bottom(rawHandle)
    }
}
