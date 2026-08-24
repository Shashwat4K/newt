import AppKit
import CNewt
import CoreText
import NewtKit

/// Draws a terminal grid with CoreText.
///
/// Renders from `GridBuffer` — a copy — never from live core state. Metal will
/// replace this later; the snapshot boundary is what makes that a contained
/// change rather than a rewrite.
///
/// No terminal semantics live here. Inverse video and color resolution are
/// already applied by the core; this file only turns cells into pixels.
@MainActor
final class TerminalView: NSView {
    let font: TerminalFont
    private var buffer = GridBuffer()

    /// Receives input events. The view draws; the controller owns the session.
    weak var inputDelegate: TerminalInputDelegate?

    /// Text being composed by an input method, before it is committed.
    ///
    /// Held so the input system can query and replace it. It is not drawn yet —
    /// showing composition in the grid needs a preedit overlay, which is
    /// deliberately left until after the MVP.
    var markedText = ""


    /// Background painted outside the glyph cells, taken from the grid so the
    /// window matches the terminal rather than guessing a color.
    private var backgroundColor = CGColor(gray: 0.08, alpha: 1)

    init(font: TerminalFont, cols: Int, rows: Int) {
        self.font = font
        super.init(
            frame: NSRect(
                x: 0,
                y: 0,
                width: CGFloat(cols) * font.cellWidth,
                height: CGFloat(rows) * font.cellHeight
            )
        )
        wantsLayer = true
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("newt does not use storyboards")
    }

    /// Adopt a new frame of terminal state and mark what changed for redraw.
    func apply(_ snapshot: GridSnapshot) {
        buffer.update(from: snapshot)

        if let first = buffer.cell(col: 0, row: 0) {
            backgroundColor = cgColor(first.bg)
        }

        for row in buffer.damagedRows {
            setNeedsDisplay(rect(ofRow: row))
        }
    }

    override var isOpaque: Bool { true }

    /// Current grid size, for translating mouse positions into cells.
    var gridSize: (cols: Int, rows: Int) { (buffer.cols, buffer.rows) }

    /// Grid size that fits a given pixel size.
    func gridSize(fitting size: NSSize) -> TerminalSize {
        font.geometry.gridSize(fitting: size)
    }

    /// Pixel size that exactly holds a grid, for snapping the window so no
    /// partial row or column is left over.
    func pixelSize(for grid: TerminalSize) -> NSSize {
        font.geometry.pixelSize(for: grid)
    }

    /// Where the cursor is on screen, for positioning the IME candidate window.
    var cursorRectInView: NSRect {
        NSRect(
            x: CGFloat(buffer.cursor.col) * font.cellWidth,
            y: bounds.height - CGFloat(Int(buffer.cursor.row) + 1) * font.cellHeight,
            width: font.cellWidth,
            height: font.cellHeight
        )
    }

    override func draw(_ dirtyRect: NSRect) {
        guard let context = NSGraphicsContext.current?.cgContext else { return }

        // AppKit hands out CGRect.infinite on some paths (cacheDisplay among
        // them), so every rect is clamped to the view before it is used.
        let paintRect = dirtyRect.intersection(bounds)
        context.setFillColor(backgroundColor)
        context.fill(paintRect)

        guard buffer.cols > 0, buffer.rows > 0 else { return }

        // Only the rows the dirty rect actually touches.
        for row in rows(intersecting: paintRect) {
            drawBackgrounds(row: row, in: context)
            drawGlyphs(row: row, in: context)
        }

        drawCursor(in: context, dirtyRect: paintRect)
    }

    // MARK: - Drawing

    /// Fill background runs, merging adjacent cells that share a color so a
    /// full-width bar is one fill rather than one per cell.
    private func drawBackgrounds(row: Int, in context: CGContext) {
        var runStart = 0
        var runColor: NewtColor?

        func flush(end: Int) {
            guard let color = runColor, end > runStart else { return }
            context.setFillColor(cgColor(color))
            context.fill(
                NSRect(
                    x: CGFloat(runStart) * font.cellWidth,
                    y: rect(ofRow: row).minY,
                    width: CGFloat(end - runStart) * font.cellWidth,
                    height: font.cellHeight
                )
            )
        }

        for col in 0..<buffer.cols {
            guard let cell = buffer.cell(col: col, row: row) else { continue }
            if let current = runColor, sameColor(current, cell.bg) { continue }
            flush(end: col)
            runStart = col
            runColor = cell.bg
        }
        flush(end: buffer.cols)
    }

    private func drawGlyphs(row: Int, in context: CGContext) {
        let baselineY = rect(ofRow: row).minY + font.baseline

        for col in 0..<buffer.cols {
            guard let cell = buffer.cell(col: col, row: row) else { continue }
            // Spacers carry no glyph; the wide character before them covers
            // this column.
            if cell.flags & UInt16(NEWT_FLAG_WIDE_SPACER) != 0 { continue }
            if cell.flags & UInt16(NEWT_FLAG_HIDDEN) != 0 { continue }
            guard let character = buffer.character(at: cell), character != " " else {
                drawDecorations(cell: cell, col: col, row: row, in: context)
                continue
            }

            draw(
                character: character,
                cell: cell,
                at: CGPoint(x: CGFloat(col) * font.cellWidth, y: baselineY),
                color: cgColor(cell.fg),
                in: context
            )
            drawDecorations(cell: cell, col: col, row: row, in: context)
        }
    }

    /// Draw one cell's character.
    ///
    /// A `CTLine` is used rather than raw glyph lookup so that font fallback,
    /// emoji, and multi-scalar clusters all render — correctness first; the
    /// Metal renderer is where the glyph atlas belongs.
    private func draw(
        character: Character,
        cell: NewtCell,
        at origin: CGPoint,
        color: CGColor,
        in context: CGContext
    ) {
        let isBold = cell.flags & UInt16(NEWT_FLAG_BOLD) != 0
        let isItalic = cell.flags & UInt16(NEWT_FLAG_ITALIC) != 0

        let attributes: [NSAttributedString.Key: Any] = [
            .font: font.font(bold: isBold, italic: isItalic),
            .foregroundColor: color,
        ]
        let line = CTLineCreateWithAttributedString(
            NSAttributedString(string: String(character), attributes: attributes)
        )

        context.textMatrix = .identity
        context.textPosition = origin
        CTLineDraw(line, context)
    }

    /// Underlines and strikeout, which are attributes of the cell rather than
    /// of the glyph.
    private func drawDecorations(cell: NewtCell, col: Int, row: Int, in context: CGContext) {
        let rowRect = rect(ofRow: row)
        let x = CGFloat(col) * font.cellWidth
        let thickness: CGFloat = 1

        let underlines =
            UInt16(NEWT_FLAG_UNDERLINE) | UInt16(NEWT_FLAG_DOUBLE_UNDERLINE)
            | UInt16(NEWT_FLAG_UNDERCURL) | UInt16(NEWT_FLAG_DOTTED_UNDERLINE)
            | UInt16(NEWT_FLAG_DASHED_UNDERLINE)

        if cell.flags & underlines != 0 {
            context.setFillColor(cgColor(cell.fg))
            context.fill(
                NSRect(
                    x: x,
                    y: rowRect.minY + font.baseline - 2,
                    width: font.cellWidth,
                    height: thickness
                )
            )
            // The distinct underline styles are drawn as a plain rule for now;
            // the shapes belong with the glyph atlas in the Metal renderer.
        }

        if cell.flags & UInt16(NEWT_FLAG_STRIKEOUT) != 0 {
            context.setFillColor(cgColor(cell.fg))
            context.fill(
                NSRect(
                    x: x,
                    y: rowRect.midY,
                    width: font.cellWidth,
                    height: thickness
                )
            )
        }
    }

    private func drawCursor(in context: CGContext, dirtyRect: NSRect) {
        guard buffer.cursor.visible else { return }

        let col = Int(buffer.cursor.col)
        let row = Int(buffer.cursor.row)
        guard col < buffer.cols, row < buffer.rows else { return }

        let cellRect = NSRect(
            x: CGFloat(col) * font.cellWidth,
            y: rect(ofRow: row).minY,
            width: font.cellWidth,
            height: font.cellHeight
        )
        guard cellRect.intersects(dirtyRect) else { return }

        let cell = buffer.cell(col: col, row: row)
        let cursorColor = cell.map { cgColor($0.fg) } ?? CGColor(gray: 1, alpha: 1)
        context.setFillColor(cursorColor)

        switch buffer.cursor.shape {
        case UInt8(NEWT_CURSOR_BLOCK):
            context.fill(cellRect)
            // Redraw the glyph in the background color so it stays readable
            // under a filled block.
            if let cell, let character = buffer.character(at: cell) {
                draw(
                    character: character,
                    cell: cell,
                    at: CGPoint(x: cellRect.minX, y: cellRect.minY + font.baseline),
                    color: cgColor(cell.bg),
                    in: context
                )
            }
        case UInt8(NEWT_CURSOR_BEAM):
            context.fill(
                NSRect(x: cellRect.minX, y: cellRect.minY, width: 1, height: cellRect.height)
            )
        case UInt8(NEWT_CURSOR_UNDERLINE):
            context.fill(
                NSRect(x: cellRect.minX, y: cellRect.minY, width: cellRect.width, height: 1)
            )
        case UInt8(NEWT_CURSOR_HOLLOW_BLOCK):
            context.setStrokeColor(cursorColor)
            context.stroke(cellRect.insetBy(dx: 0.5, dy: 0.5), width: 1)
        default:
            break
        }
    }

    // MARK: - Geometry

    /// Row 0 is at the top; the view is not flipped, so it maps to the highest
    /// y in the frame.
    private func rect(ofRow row: Int) -> NSRect {
        NSRect(
            x: 0,
            y: bounds.height - CGFloat(row + 1) * font.cellHeight,
            width: bounds.width,
            height: font.cellHeight
        )
    }

    /// Rows touched by a rect, in row-index order.
    ///
    /// The rect is clamped first: an unclamped infinite rect converts to a
    /// Double outside `Int`'s range and traps.
    private func rows(intersecting rect: NSRect) -> Range<Int> {
        let clamped = rect.intersection(bounds)
        guard !clamped.isNull, !clamped.isEmpty, font.cellHeight > 0 else { return 0..<0 }

        let first = Int(((bounds.height - clamped.maxY) / font.cellHeight).rounded(.down))
        let end = Int(((bounds.height - clamped.minY) / font.cellHeight).rounded(.up))

        let lower = max(0, first)
        let upper = min(buffer.rows, end)
        return lower < upper ? lower..<upper : 0..<0
    }

    private func cgColor(_ color: NewtColor) -> CGColor {
        CGColor(
            srgbRed: CGFloat(color.r) / 255,
            green: CGFloat(color.g) / 255,
            blue: CGFloat(color.b) / 255,
            alpha: 1
        )
    }

    private func sameColor(_ lhs: NewtColor, _ rhs: NewtColor) -> Bool {
        lhs.r == rhs.r && lhs.g == rhs.g && lhs.b == rhs.b
    }
}
