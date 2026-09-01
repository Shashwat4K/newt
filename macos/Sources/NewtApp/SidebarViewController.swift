import AppKit
import NewtKit

/// The tab sidebar: an outline view over the window's `TabTree`.
///
/// `NSOutlineView` rather than a hand-drawn view, for the same reason Phase 7
/// chose `NSSplitView` for the pane tree — indentation, disclosure triangles,
/// expand/collapse, arrow-key navigation, source-list styling and VoiceOver are
/// what an outline view *is*. Hand-drawing means reimplementing hit-testing and
/// accessibility for no gain, and Story 3's nesting is exactly its use case.
@MainActor
final class SidebarViewController: NSObject, NSOutlineViewDataSource, NSOutlineViewDelegate {
    let scrollView = NSScrollView()
    private let outlineView = NSOutlineView()

    /// The model. The window controller owns it and pushes it here.
    var tree = TabTree()
    /// Identity → the controller that stands for it.
    var controllers: [TabID: TerminalTabController] = [:]

    var onSelect: ((TabID) -> Void)?

    /// Set for the whole of `reload()`.
    ///
    /// Rebuilding rows expands items and restores the selection, and both of
    /// those make `NSOutlineView` call back — into `reloadData` reentrantly,
    /// which it explicitly does not support, and into the selection delegate,
    /// which would send us straight back round through tab activation.
    private var isReloading = false

    override init() {
        super.init()

        let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("tab"))
        column.resizingMask = .autoresizingMask
        outlineView.addTableColumn(column)
        outlineView.outlineTableColumn = column
        outlineView.headerView = nil
        // The standard macOS sidebar look. `style` rather than the
        // deprecated `selectionHighlightStyle = .sourceList`.
        outlineView.style = .sourceList
        outlineView.allowsEmptySelection = false
        outlineView.allowsMultipleSelection = false
        outlineView.indentationPerLevel = 14
        outlineView.rowSizeStyle = .custom
        outlineView.autoresizesOutlineColumn = false
        outlineView.dataSource = self
        outlineView.delegate = self

        scrollView.documentView = outlineView
        scrollView.hasVerticalScroller = true
        scrollView.drawsBackground = false
        scrollView.autohidesScrollers = true
    }

    // MARK: - Updating

    /// Rebuild the rows, keeping the selection and what is expanded.
    func reload() {
        guard !isReloading else { return }
        isReloading = true
        defer { isReloading = false }

        outlineView.reloadData()

        // New tabs are nested under a parent that must be open for them to be
        // visible at all, so expansion is unconditional rather than remembered.
        for (node, _) in tree.flattened() where !tree.children(of: node.id).isEmpty {
            if let controller = controllers[node.id] {
                outlineView.expandItem(controller, expandChildren: false)
            }
        }

        applySelection()
    }

    /// Redraw one row in place. Used by the status ticker, which runs
    /// constantly — a full `reload()` at 5 Hz would fight the selection.
    func reloadRow(_ id: TabID) {
        guard let controller = controllers[id] else { return }
        let row = outlineView.row(forItem: controller)
        guard row >= 0 else { return }
        if let view = outlineView.view(atColumn: 0, row: row, makeIfNecessary: false) as? TabRowView
        {
            configure(view, for: controller)
        }
    }

    private func applySelection() {
        guard let selected = tree.selected, let controller = controllers[selected] else { return }
        let row = outlineView.row(forItem: controller)
        guard row >= 0 else { return }

        guard outlineView.selectedRow != row else { return }
        outlineView.selectRowIndexes(IndexSet(integer: row), byExtendingSelection: false)
    }

    private func configure(_ view: TabRowView, for controller: TerminalTabController) {
        let accentIndex = tree.node(controller.id)?.accentIndex ?? 0
        let metadata = controller.focusedPane?.session.metadata ?? SessionMetadata()
        view.configure(tab: controller, accentIndex: accentIndex, metadata: metadata)
    }

    // MARK: - NSOutlineViewDataSource

    func outlineView(_ outlineView: NSOutlineView, numberOfChildrenOfItem item: Any?) -> Int {
        ids(under: item).count
    }

    func outlineView(_ outlineView: NSOutlineView, child index: Int, ofItem item: Any?) -> Any {
        let ids = ids(under: item)
        guard ids.indices.contains(index), let controller = controllers[ids[index]] else {
            // The data source must return something; an empty placeholder tab
            // is better than trapping if the tree and the map ever disagree.
            return NSNull()
        }
        return controller
    }

    func outlineView(_ outlineView: NSOutlineView, isItemExpandable item: Any) -> Bool {
        guard let controller = item as? TerminalTabController else { return false }
        return !tree.children(of: controller.id).isEmpty
    }

    private func ids(under item: Any?) -> [TabID] {
        guard let controller = item as? TerminalTabController else { return tree.roots }
        return tree.children(of: controller.id)
    }

    // MARK: - NSOutlineViewDelegate

    func outlineView(_ outlineView: NSOutlineView, viewFor column: NSTableColumn?, item: Any)
        -> NSView?
    {
        guard let controller = item as? TerminalTabController else { return nil }
        // A fresh row rather than a recycled one: the badge is built for a
        // specific agent, and a sidebar holds tens of rows, not thousands.
        let view = TabRowView(kind: controller.kind)
        configure(view, for: controller)
        return view
    }

    func outlineView(_ outlineView: NSOutlineView, heightOfRowByItem item: Any) -> CGFloat {
        guard let controller = item as? TerminalTabController else { return 28 }
        // Agent rows carry a subtitle; shell rows do not, and giving them the
        // taller height would leave the list looking gap-toothed.
        return controller.kind.agent == nil ? 28 : 40
    }

    func outlineViewSelectionDidChange(_ notification: Notification) {
        guard !isReloading else { return }
        let row = outlineView.selectedRow
        guard row >= 0,
            let controller = outlineView.item(atRow: row) as? TerminalTabController
        else { return }
        onSelect?(controller.id)
    }

    /// Every row is a real tab; none is a group header.
    func outlineView(_ outlineView: NSOutlineView, isGroupItem item: Any) -> Bool { false }
}
