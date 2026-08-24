import CNewt

/// Assemble a cell's primary codepoint and its combining marks into one
/// character. Returns nil for spacers and empty cells, which must not be drawn.
func assembleCharacter(codepoint: UInt32, marks: some Sequence<UInt32>) -> Character? {
    guard codepoint != 0, let scalar = Unicode.Scalar(codepoint) else { return nil }

    var view = String.UnicodeScalarView()
    view.append(scalar)
    for mark in marks {
        if let mark = Unicode.Scalar(mark) {
            view.append(mark)
        }
    }
    // A cluster can still decompose into several characters if the marks do not
    // combine; the first is the one anchored to this cell.
    return String(view).first
}

/// A render-side copy of the terminal grid.
///
/// The renderer must never read live core state — a snapshot borrows session
/// memory that is only valid for the duration of one call, and drawing happens
/// later. This holds a copy the view can draw from at any time.
///
/// The cell and combining arrays are copied wholesale each update: they are
/// plain C structs, so this is a memcpy, and it keeps combining offsets
/// consistent with the cells that point into them. Damage is used to decide
/// which rows to *redraw*, which is where the real cost is.
public struct GridBuffer {
    public private(set) var cols: Int = 0
    public private(set) var rows: Int = 0
    public private(set) var cells: [NewtCell] = []
    public private(set) var combining: [UInt32] = []
    public private(set) var cursor = NewtCursor(col: 0, row: 0, shape: 0, visible: false)

    /// Rows that changed in the most recent update and need redrawing.
    public private(set) var damagedRows: [Int] = []

    public init() {}

    /// Copy a snapshot in, recording which rows need redrawing.
    public mutating func update(from snapshot: GridSnapshot) {
        let resized = snapshot.cols != cols || snapshot.rows != rows
        let previousCursor = cursor

        cols = snapshot.cols
        rows = snapshot.rows
        cells = Array(snapshot.cells)
        combining = Array(snapshot.combining)
        cursor = snapshot.cursor

        if resized || snapshot.isFullyDamaged {
            damagedRows = Array(0..<rows)
            return
        }

        var rowsToRedraw = Set<Int>()
        for span in snapshot.damage where span.row < UInt32(rows) {
            rowsToRedraw.insert(Int(span.row))
        }

        // The cell the cursor just left has to be repainted even when the
        // engine reports no damage for it.
        if previousCursor.visible, Int(previousCursor.row) < rows {
            rowsToRedraw.insert(Int(previousCursor.row))
        }
        if cursor.visible, Int(cursor.row) < rows {
            rowsToRedraw.insert(Int(cursor.row))
        }

        damagedRows = rowsToRedraw.sorted()
    }

    public func cell(col: Int, row: Int) -> NewtCell? {
        guard col >= 0, row >= 0, col < cols, row < rows else { return nil }
        return cells[row * cols + col]
    }

    /// The full character for a cell, including combining marks.
    public func character(at cell: NewtCell) -> Character? {
        let start = Int(cell.combining_offset)
        let count = Int(cell.combining_len)
        let marks: ArraySlice<UInt32> =
            count > 0 && start + count <= combining.count
            ? combining[start..<(start + count)]
            : []
        return assembleCharacter(codepoint: cell.codepoint, marks: marks)
    }

    /// The whole visible screen as text, trailing blank lines trimmed. For
    /// tests and debugging; the renderer reads cells directly.
    public func text() -> String {
        var lines = (0..<rows).map { text(row: $0) }
        while let last = lines.last, last.isEmpty { lines.removeLast() }
        return lines.joined(separator: "\n")
    }

    /// One row as text, trailing blanks trimmed. For tests and debugging.
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
}
