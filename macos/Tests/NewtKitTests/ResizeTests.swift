import CoreGraphics
import XCTest
@testable import NewtKit

final class GridGeometryTests: XCTestCase {
    private let geometry = GridGeometry(cellWidth: 8, cellHeight: 17)

    func testExactSizeYieldsExactGrid() {
        let grid = geometry.gridSize(fitting: CGSize(width: 800, height: 340))
        XCTAssertEqual(grid, TerminalSize(cols: 100, rows: 20))
    }

    /// A partial cell must be dropped, not rounded up — rounding up gives a
    /// column that can never be fully drawn.
    func testPartialCellsAreDropped() {
        let grid = geometry.gridSize(fitting: CGSize(width: 807, height: 356))
        XCTAssertEqual(grid, TerminalSize(cols: 100, rows: 20))
    }

    func testTinyAndDegenerateSizesStillYieldAUsableGrid() {
        XCTAssertEqual(geometry.gridSize(fitting: .zero), TerminalSize(cols: 1, rows: 1))
        XCTAssertEqual(
            geometry.gridSize(fitting: CGSize(width: 3, height: 3)),
            TerminalSize(cols: 1, rows: 1)
        )
        XCTAssertEqual(
            geometry.gridSize(fitting: CGSize(width: -100, height: -100)),
            TerminalSize(cols: 1, rows: 1)
        )
    }

    /// Snapping has to be stable: feeding a snapped size back in must give the
    /// same grid, or a window creeps by a row every time it is dragged.
    func testSnappingIsStable() {
        for width in stride(from: 100.0, through: 900.0, by: 37.0) {
            for height in stride(from: 100.0, through: 600.0, by: 41.0) {
                let size = CGSize(width: width, height: height)
                let grid = geometry.gridSize(fitting: size)
                let snapped = geometry.pixelSize(for: grid)

                XCTAssertEqual(
                    geometry.gridSize(fitting: snapped),
                    grid,
                    "snapping \(size) to \(snapped) changed the grid"
                )
                XCTAssertLessThanOrEqual(snapped.width, max(size.width, geometry.cellWidth))
                XCTAssertLessThanOrEqual(snapped.height, max(size.height, geometry.cellHeight))
            }
        }
    }

    func testZeroCellSizeCannotDivideByZero() {
        let degenerate = GridGeometry(cellWidth: 0, cellHeight: 0)
        XCTAssertEqual(
            degenerate.gridSize(fitting: CGSize(width: 100, height: 100)),
            TerminalSize(cols: 100, rows: 100)
        )
    }
}

/// Reflow through the real session, so the policy the core inherits is checked
/// from the side that depends on it.
final class ReflowTests: XCTestCase {
    func testNarrowingRewrapsAndWideningRestores() throws {
        let session = try TerminalSession(
            size: TerminalSize(cols: 40, rows: 6),
            shell: "/bin/sh"
        )
        try session.write("printf 'abcdefghijklmnopqrstuvwxyz0123456789'\n")

        let deadline = Date().addingTimeInterval(5)
        var appeared = false
        while Date() < deadline, !appeared {
            Thread.sleep(forTimeInterval: 0.02)
            appeared = try session.withSnapshot { $0.text().contains("abcdefghij") }
        }
        XCTAssertTrue(appeared, "output never arrived")

        try session.resize(to: TerminalSize(cols: 20, rows: 6))
        let narrow = try session.withSnapshot { snapshot -> String in
            XCTAssertEqual(snapshot.cols, 20)
            return snapshot.text()
        }
        XCTAssertTrue(
            narrow.contains("0123456789"),
            "content was lost when narrowing: \(narrow)"
        )

        try session.resize(to: TerminalSize(cols: 40, rows: 6))
        let wide = try session.withSnapshot { $0.text() }
        XCTAssertTrue(
            wide.contains("abcdefghijklmnopqrstuvwxyz0123456789"),
            "widening did not restore the unwrapped line: \(wide)"
        )
    }
}
