import CoreGraphics

/// Converts between pixel sizes and grid sizes.
///
/// Small enough to look obvious and wrong often enough to be worth testing: an
/// off-by-one here shows up as a permanently clipped last column, or a window
/// that grows a row every time it is dragged.
public struct GridGeometry: Equatable, Sendable {
    public let cellWidth: CGFloat
    public let cellHeight: CGFloat

    public init(cellWidth: CGFloat, cellHeight: CGFloat) {
        self.cellWidth = max(1, cellWidth)
        self.cellHeight = max(1, cellHeight)
    }

    /// Largest grid that fits inside a pixel size.
    ///
    /// Rounded down, never below 1x1: a zero-sized grid is meaningless, and
    /// programs cope with a tiny terminal far better than an impossible one.
    public func gridSize(fitting size: CGSize) -> TerminalSize {
        let cols = Int((size.width / cellWidth).rounded(.down))
        let rows = Int((size.height / cellHeight).rounded(.down))
        return TerminalSize(
            cols: UInt16(clamping: max(1, cols)),
            rows: UInt16(clamping: max(1, rows))
        )
    }

    /// Pixel size that exactly holds a grid, for snapping a window so no
    /// partial row or column is left over.
    public func pixelSize(for grid: TerminalSize) -> CGSize {
        CGSize(
            width: CGFloat(grid.cols) * cellWidth,
            height: CGFloat(grid.rows) * cellHeight
        )
    }
}
