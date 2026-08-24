import CNewt

/// A borrowed view of one frame of terminal state.
///
/// The buffers belong to the session and stay valid only for the duration of
/// the `withSnapshot` call that produced this view — copying the whole grid
/// into Swift arrays every frame would undo the point of the flat layout.
public struct GridSnapshot {
    public let cols: Int
    public let rows: Int
    public let cells: UnsafeBufferPointer<NewtCell>
    public let combining: UnsafeBufferPointer<UInt32>
    public let cursor: NewtCursor
    /// Rows changed since the previous snapshot. Ignore when `isFullyDamaged`.
    public let damage: UnsafeBufferPointer<NewtDamagedRow>
    public let isFullyDamaged: Bool
    /// Lines the viewport is scrolled back into history.
    public let displayOffset: UInt32
    /// Lines currently held in scrollback.
    public let historyLength: UInt32

    init(_ raw: NewtSnapshot) {
        cols = Int(raw.cols)
        rows = Int(raw.rows)
        cells = UnsafeBufferPointer(start: raw.cells, count: Int(raw.cell_count))
        combining = UnsafeBufferPointer(start: raw.combining, count: Int(raw.combining_count))
        cursor = raw.cursor
        damage = UnsafeBufferPointer(start: raw.damage, count: Int(raw.damage_count))
        isFullyDamaged = raw.full_damage
        displayOffset = raw.display_offset
        historyLength = raw.history_len
    }

    /// Cell at a position, or nil when out of bounds.
    public func cell(col: Int, row: Int) -> NewtCell? {
        guard col >= 0, row >= 0, col < cols, row < rows else { return nil }
        return cells[row * cols + col]
    }

    /// The full character for a cell: its primary codepoint plus any combining
    /// marks held in the side table. Returns nil for spacers and empty cells,
    /// which must not be drawn.
    public func character(at cell: NewtCell) -> Character? {
        guard cell.codepoint != 0, let scalar = Unicode.Scalar(cell.codepoint) else { return nil }

        let start = Int(cell.combining_offset)
        let count = Int(cell.combining_len)
        guard count > 0 else { return Character(scalar) }

        var view = String.UnicodeScalarView()
        view.append(scalar)
        for index in start..<(start + count) where index < combining.count {
            if let mark = Unicode.Scalar(combining[index]) {
                view.append(mark)
            }
        }
        // A cluster can still decompose into several characters if the marks do
        // not combine; the first is the one anchored to this cell.
        return String(view).first
    }

    /// One row rendered as text, trailing blanks trimmed. For tests and
    /// debugging — the renderer reads cells directly.
    public func text(row: Int) -> String {
        guard row >= 0, row < rows else { return "" }
        var line = ""
        for col in 0..<cols {
            guard let cell = cell(col: col, row: row) else { continue }
            if cell.flags & UInt16(NEWT_FLAG_WIDE_SPACER) != 0 { continue }
            line.append(character(at: cell) ?? " ")
        }
        while line.hasSuffix(" ") { line.removeLast() }
        return line
    }

    /// The whole visible screen as text, trailing blank lines trimmed.
    public func text() -> String {
        var lines = (0..<rows).map { text(row: $0) }
        while let last = lines.last, last.isEmpty { lines.removeLast() }
        return lines.joined(separator: "\n")
    }
}
