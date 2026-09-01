import XCTest

@testable import NewtKit

/// The tab tree's invariants: display order, nesting, subtree removal, and a
/// selection that survives a close. These are exactly the cases that would
/// otherwise only be found by clicking.
final class TabTreeTests: XCTestCase {
    func testTheFirstTabIsSelectedAndLaterOnesDoNotStealIt() {
        var tree = TabTree()
        let first = tree.insert(kind: .shell)
        let second = tree.insert(kind: .shell)

        XCTAssertEqual(tree.selected, first)
        XCTAssertNotEqual(tree.selected, second)
        XCTAssertEqual(tree.count, 2)
    }

    func testChildrenSitDirectlyBelowTheirParentInDisplayOrder() {
        var tree = TabTree()
        let parent = tree.insert(kind: .agent(.claude))
        let child = tree.insert(kind: .shell, under: parent)
        let sibling = tree.insert(kind: .shell)

        let order = tree.flattened()
        XCTAssertEqual(order.map(\.node.id), [parent, child, sibling])
        XCTAssertEqual(order.map(\.depth), [0, 1, 0])
    }

    func testNestingGoesDeeperThanOneLevel() {
        var tree = TabTree()
        let root = tree.insert(kind: .shell)
        let child = tree.insert(kind: .shell, under: root)
        let grandchild = tree.insert(kind: .shell, under: child)

        XCTAssertEqual(tree.depth(of: grandchild), 2)
        XCTAssertEqual(tree.flattened().map(\.depth), [0, 1, 2])
        XCTAssertEqual(tree.subtree(of: root), [root, child, grandchild])
    }

    func testRemovingAParentTakesItsWholeSubtree() {
        var tree = TabTree()
        let parent = tree.insert(kind: .shell)
        let child = tree.insert(kind: .shell, under: parent)
        let grandchild = tree.insert(kind: .shell, under: child)
        let survivor = tree.insert(kind: .shell)

        let removed = tree.remove(parent)

        // Parents before children: the caller tears down sessions in this order.
        XCTAssertEqual(removed, [parent, child, grandchild])
        XCTAssertEqual(tree.count, 1)
        XCTAssertNil(tree.node(child))
        XCTAssertNotNil(tree.node(survivor))
    }

    func testSelectionMovesToTheNeighbourAboveWhenTheSelectedTabIsRemoved() {
        var tree = TabTree()
        let first = tree.insert(kind: .shell)
        let second = tree.insert(kind: .shell)
        let third = tree.insert(kind: .shell)
        tree.select(second)

        tree.remove(second)

        XCTAssertEqual(tree.selected, first)
        XCTAssertNotNil(tree.node(third))
    }

    func testSelectionFallsForwardWhenNothingSurvivesAboveIt() {
        var tree = TabTree()
        let parent = tree.insert(kind: .shell)
        tree.insert(kind: .shell, under: parent)
        let survivor = tree.insert(kind: .shell)
        tree.select(parent)

        // Removing the first root takes its child with it, so there is nothing
        // above the removal to fall back to.
        tree.remove(parent)

        XCTAssertEqual(tree.selected, survivor)
    }

    func testRemovingAnUnselectedTabLeavesTheSelectionAlone() {
        var tree = TabTree()
        let first = tree.insert(kind: .shell)
        let second = tree.insert(kind: .shell)
        tree.select(first)

        tree.remove(second)

        XCTAssertEqual(tree.selected, first)
    }

    func testEmptyingTheTreeClearsTheSelection() {
        var tree = TabTree()
        let only = tree.insert(kind: .shell)

        tree.remove(only)

        XCTAssertNil(tree.selected)
        XCTAssertTrue(tree.isEmpty)
    }

    func testSelectionCyclesThroughDisplayOrderIncludingChildren() {
        var tree = TabTree()
        let parent = tree.insert(kind: .shell)
        let child = tree.insert(kind: .shell, under: parent)
        let sibling = tree.insert(kind: .shell)

        XCTAssertEqual(tree.selected, parent)
        tree.selectNext()
        XCTAssertEqual(tree.selected, child)
        tree.selectNext()
        XCTAssertEqual(tree.selected, sibling)
        // Wraps rather than stopping at the end.
        tree.selectNext()
        XCTAssertEqual(tree.selected, parent)
        tree.selectPrevious()
        XCTAssertEqual(tree.selected, sibling)
    }

    func testSelectByDisplayIndexIgnoresOutOfRange() {
        var tree = TabTree()
        let parent = tree.insert(kind: .shell)
        let child = tree.insert(kind: .shell, under: parent)

        tree.select(displayIndex: 1)
        XCTAssertEqual(tree.selected, child)

        tree.select(displayIndex: 9)
        XCTAssertEqual(tree.selected, child, "an out-of-range ⌘N must do nothing, not trap")
    }

    func testRootsCycleThePaletteAndChildrenInheritTheirParents() {
        var tree = TabTree()
        var roots: [TabID] = []
        for _ in 0..<(TabTree.accentCount + 1) {
            roots.append(tree.insert(kind: .shell))
        }

        let indices = roots.map { tree.node($0)!.accentIndex }
        XCTAssertEqual(Array(indices.prefix(TabTree.accentCount)), Array(0..<TabTree.accentCount))
        XCTAssertEqual(indices.last, 0, "the palette wraps")

        let child = tree.insert(kind: .shell, under: roots[3])
        XCTAssertEqual(tree.node(child)!.accentIndex, tree.node(roots[3])!.accentIndex)
    }

    func testClosingATabDoesNotRecycleItsAccentOntoTheNextOne() {
        var tree = TabTree()
        let first = tree.insert(kind: .shell)
        let firstAccent = tree.node(first)!.accentIndex
        tree.remove(first)

        let next = tree.insert(kind: .shell)
        XCTAssertNotEqual(
            tree.node(next)!.accentIndex,
            firstAccent,
            "reusing the colour of a tab that just closed reads as the same tab"
        )
    }

    func testAnUnknownParentBecomesARootRatherThanAnOrphan() {
        var tree = TabTree()
        let stale = TabID()
        let id = tree.insert(kind: .shell, under: stale)

        XCTAssertEqual(tree.roots, [id])
        XCTAssertNil(tree.node(id)!.parent)
    }

    func testRemovingAnUnknownTabIsANoOp() {
        var tree = TabTree()
        let only = tree.insert(kind: .shell)

        XCTAssertEqual(tree.remove(TabID()), [])
        XCTAssertEqual(tree.count, 1)
        XCTAssertEqual(tree.selected, only)
    }

    func testTitlesAreCarriedPerTab() {
        var tree = TabTree()
        let id = tree.insert(kind: .agent(.claude))
        XCTAssertNil(tree.node(id)!.title)

        tree.setTitle("newt refactor", for: id)
        XCTAssertEqual(tree.node(id)!.title, "newt refactor")
        XCTAssertTrue(tree.node(id)!.isAgent)
        XCTAssertEqual(tree.node(id)!.kind.agent, .claude)
    }
}
