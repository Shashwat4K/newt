import AppKit
import CNewt
import Foundation
import NewtKit

/// One window: a sidebar of tabs on the left, the selected tab's pane tree on
/// the right, and the find bar floating over it.
///
/// Phase 7 made this class the window, the tab, *and* the pane tree at once,
/// because native window tabbing meant one window was one tab. Stage 2 drops
/// native tabbing — see the plan for why — so those three become three types.
/// What is left here is the window and the composition; a tab owns its panes.
///
/// It stays an `NSWindowController`: menu commands reach their target through
/// the responder chain, and a plain `NSObject` is not in it. That was a
/// Phase-7 finding and it is still load-bearing.
@MainActor
final class TerminalWindowController: NSWindowController, NSWindowDelegate {
    /// Kept as a non-optional alongside `NSWindowController.window`, which is
    /// optional and would need unwrapping at every use.
    let hostWindow: NSWindow

    private let font: TerminalFont
    /// `NSSplitViewController` rather than a hand-laid `NSSplitView`: it owns
    /// the sidebar's minimum and maximum thickness, its collapsed state, the
    /// standard vibrant sidebar material, and `toggleSidebar(_:)`. Laying the
    /// two halves out by hand meant setting frames that AppKit then re-derived,
    /// and the pane area came out a sidebar's width too narrow.
    private let bodyController = NSSplitViewController()
    private let sidebar = SidebarViewController()
    /// Holds the selected tab's pane tree. The find bar floats above it.
    private let paneContainer = NSView()
    private let findBar = FindBar()

    private var tree = TabTree()
    private var tabs: [TabID: TerminalTabController] = [:]

    /// Grid size used for tabs and panes created after the first.
    private let defaultSize: TerminalSize
    /// Program plain shell tabs in this window run. `nil` is the login shell.
    private let shell: String?

    private static let sidebarWidth: CGFloat = 208
    private static let sidebarMinWidth: CGFloat = 150
    private static let sidebarMaxWidth: CGFloat = 340

    var selectedTab: TerminalTabController? {
        tree.selected.flatMap { tabs[$0] }
    }

    /// The selected tab's panes. `OfflineRender` reads this.
    var panes: [TerminalPaneController] { selectedTab?.panes ?? [] }

    /// Frames of the pieces that decide how much room the grid gets. Reported
    /// by `--render-to --panes`, because a collapsed pane is a layout problem
    /// and the grid sizes alone do not say which view lost its space.
    var debugLayout: String {
        func size(_ view: NSView) -> String {
            "\(Int(view.frame.width))x\(Int(view.frame.height))"
        }
        let tabContent = selectedTab.map { size($0.contentView) } ?? "-"
        return "split \(size(bodyController.view)) sidebar \(size(sidebar.scrollView)) "
            + "container \(size(paneContainer)) tab \(tabContent)"
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("newt does not use storyboards")
    }

    init(font: TerminalFont, cols: UInt16, rows: UInt16, shell: String? = nil) throws {
        self.font = font
        self.shell = shell
        defaultSize = TerminalSize(cols: cols, rows: rows)

        let gridSize = font.geometry.pixelSize(for: defaultSize)
        let contentRect = NSRect(
            x: 0,
            y: 0,
            width: gridSize.width + Self.sidebarWidth,
            height: gridSize.height
        )

        hostWindow = NSWindow(
            contentRect: contentRect,
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false
        )
        hostWindow.title = "newt"
        hostWindow.contentMinSize = NSSize(
            width: font.geometry.pixelSize(for: TerminalSize(cols: 20, rows: 5)).width
                + Self.sidebarMinWidth,
            height: font.geometry.pixelSize(for: TerminalSize(cols: 20, rows: 5)).height
        )
        // Explicitly disallowed, not left to default: `.automatic` puts "Merge
        // All Windows" back in the Window menu and silently reintroduces native
        // tabs behind the sidebar.
        hostWindow.tabbingMode = .disallowed

        let sidebarPane = NSViewController()
        // The item's starting thickness comes from its view's frame; without
        // this the sidebar opens collapsed to `minimumThickness`.
        sidebar.scrollView.frame = NSRect(
            x: 0,
            y: 0,
            width: Self.sidebarWidth,
            height: contentRect.height
        )
        sidebarPane.view = sidebar.scrollView
        let sidebarItem = NSSplitViewItem(sidebarWithViewController: sidebarPane)
        sidebarItem.minimumThickness = Self.sidebarMinWidth
        sidebarItem.maximumThickness = Self.sidebarMaxWidth
        sidebarItem.canCollapse = true
        // The grid takes the space a resize adds; the sidebar keeps its width.
        sidebarItem.holdingPriority = .defaultHigh

        let contentPane = NSViewController()
        paneContainer.frame = NSRect(
            x: 0,
            y: 0,
            width: gridSize.width,
            height: contentRect.height
        )
        contentPane.view = paneContainer
        let contentItem = NSSplitViewItem(viewController: contentPane)

        bodyController.splitViewItems = [sidebarItem, contentItem]
        bodyController.view.frame = contentRect

        findBar.isHidden = true
        findBar.autoresizingMask = [.minXMargin, .minYMargin]
        findBar.setFrameOrigin(
            NSPoint(
                x: paneContainer.bounds.maxX - findBar.frame.width - 12,
                y: paneContainer.bounds.maxY - findBar.frame.height - 12
            )
        )
        // Over the grid, not over the sidebar: the find bar searches the
        // focused pane, so it belongs above what it is searching.
        paneContainer.addSubview(findBar)

        hostWindow.contentViewController = bodyController
        hostWindow.setContentSize(contentRect.size)

        super.init(window: hostWindow)

        hostWindow.delegate = self

        sidebar.onSelect = { [weak self] id in self?.select(id) }

        findBar.onFind = { [weak self] query, forward in
            guard let self, let pane = selectedTab?.focusedPane else { return false }
            do {
                return try pane.session.find(query, forward: forward)
            } catch {
                pane.report(error)
                return false
            }
        }
        findBar.onClose = { [weak self] in
            guard let self else { return }
            selectedTab?.takeFocus(in: hostWindow)
        }

        try addTab(kind: .shell, under: nil, select: true)
    }

    /// Show the window and start the selected tab.
    func start(runningCommand command: String? = nil) {
        hostWindow.makeKeyAndOrderFront(nil)
        sidebar.reload()

        if let tab = selectedTab {
            tab.activate(in: paneContainer)
            tab.takeFocus(in: hostWindow)
            if let command, let pane = tab.focusedPane {
                try? pane.session.write("\(command)\n")
            }
        }
    }

    func stop() {
        for tab in tabs.values {
            tab.stop()
        }
    }

    /// Called by the app-wide status ticker. Covers background tabs too.
    func pollStatus() {
        for tab in tabs.values {
            tab.pollStatus()
        }
    }

    // MARK: - Tabs

    @discardableResult
    private func addTab(kind: TabKind, under parent: TabID?, select shouldSelect: Bool) throws
        -> TabID
    {
        let id = TabID()
        let controller = try TerminalTabController(
            id: id,
            kind: kind,
            font: font,
            size: currentGridSize(),
            spec: sessionSpec(for: kind)
        )
        tabs[id] = controller
        tree.insert(kind: kind, under: parent, id: id)

        controller.onFocusChange = { [weak self] tab in
            guard let self else { return }
            hostWindow.title = tab.displayTitle
            sidebar.reloadRow(tab.id)
        }
        controller.onStatusChange = { [weak self] tab in
            guard let self else { return }
            if tree.selected == tab.id {
                hostWindow.title = tab.displayTitle
            }
            sidebar.reloadRow(tab.id)
        }
        controller.onEmpty = { [weak self] tab in self?.closeTab(tab.id) }

        sidebar.controllers = tabs
        sidebar.tree = tree

        if shouldSelect {
            select(id)
        } else {
            sidebar.reload()
        }
        return id
    }

    /// What a tab of this kind should run.
    ///
    /// The agent case names only the *kind* and where the helper lives; the
    /// core resolves the executable, writes the hooks settings, and builds the
    /// argument list. Nothing here knows what `--fork-session` means, which is
    /// the boundary rule doing its job.
    private func sessionSpec(for kind: TabKind, forkingFrom parent: String? = nil) -> SessionSpec {
        switch kind {
        case .shell:
            return SessionSpec(size: defaultSize, program: shell)
        case .agent(let agent):
            return SessionSpec(
                size: defaultSize,
                workingDirectory: FileManager.default.currentDirectoryPath,
                agent: agent,
                agentHelperPath: Self.hookHelperPath,
                agentResumeID: parent
            )
        }
    }

    /// The bundled `newt-hook`, beside this executable.
    ///
    /// Resolved on this side because knowing where a bundle keeps its
    /// executables is the shell's job. `nil` when it is missing, which starts
    /// the agent with no hooks rather than pointing Claude Code at a command
    /// that does not exist — every tool call would then run a missing program.
    static let hookHelperPath: String? = {
        guard let executable = Bundle.main.executableURL else { return nil }
        let candidate = executable.deletingLastPathComponent().appendingPathComponent("newt-hook")
        return FileManager.default.isExecutableFile(atPath: candidate.path)
            ? candidate.path : nil
    }()

    /// Grid size a new tab should start at, so it matches the window rather
    /// than the size the window was created with.
    private func currentGridSize() -> TerminalSize {
        let fitted = font.geometry.gridSize(fitting: paneContainer.bounds.size)
        return fitted.cols >= 20 && fitted.rows >= 5 ? fitted : defaultSize
    }

    /// Swap the visible tab. Sessions are never torn down by this.
    private func select(_ id: TabID) {
        guard let next = tabs[id] else { return }
        // Already showing: selecting it again would deactivate and reactivate
        // the same tab, which tears down and rebuilds its display links for
        // nothing — and, via the sidebar, recurses.
        if next.isActive { return }

        if let current = selectedTab, current !== next {
            current.deactivate()
        }
        tree.select(id)
        next.activate(in: paneContainer)
        next.synchronizeSize(to: paneContainer)
        next.takeFocus(in: hostWindow)
        hostWindow.title = next.displayTitle

        keepFindBarOnTop()
        sidebar.tree = tree
        sidebar.reload()
    }

    @objc func newTab(_ sender: Any?) {
        do {
            try addTab(kind: .shell, under: nil, select: true)
        } catch {
            reportPaneFailure(error)
        }
    }

    /// New tab running Claude Code.
    @objc func newAgentTab(_ sender: Any?) {
        do {
            try addTab(kind: .agent(.claude), under: nil, select: true)
        } catch {
            reportPaneFailure(error)
        }
    }

    /// Greyed out when the agent is not installed, rather than offered and
    /// then failing once someone picks it.
    @objc func validateMenuItem(_ item: NSMenuItem) -> Bool {
        if item.action == #selector(newAgentTab(_:)) {
            return newt_agent_available(AgentKind.claude.rawValue)
        }
        return true
    }

    @objc func closeSelectedTab(_ sender: Any?) {
        guard let id = tree.selected else { return }
        closeTab(id)
    }

    /// Close a tab and everything nested under it.
    private func closeTab(_ id: TabID) {
        let removed = tree.remove(id)
        guard !removed.isEmpty else { return }

        for removedID in removed {
            let tab = tabs.removeValue(forKey: removedID)
            tab?.deactivate()
            tab?.stop()
        }

        sidebar.controllers = tabs
        sidebar.tree = tree

        if tabs.isEmpty {
            hostWindow.close()
            return
        }

        if let selected = tree.selected {
            // The tree already chose the neighbour; activate it for real.
            tree.select(selected)
            tabs[selected]?.activate(in: paneContainer)
            tabs[selected]?.synchronizeSize(to: paneContainer)
            tabs[selected]?.takeFocus(in: hostWindow)
            hostWindow.title = tabs[selected]?.displayTitle ?? "newt"
            keepFindBarOnTop()
        }
        sidebar.reload()
    }

    @objc func selectNextTab(_ sender: Any?) {
        tree.selectNext()
        if let id = tree.selected { select(id) }
    }

    @objc func selectPreviousTab(_ sender: Any?) {
        tree.selectPrevious()
        if let id = tree.selected { select(id) }
    }

    /// ⌘1…⌘9, by position in the sidebar.
    @objc func selectTabByNumber(_ sender: Any?) {
        guard let item = sender as? NSMenuItem, let index = Int(item.keyEquivalent) else { return }
        tree.select(displayIndex: index - 1)
        if let id = tree.selected { select(id) }
    }

    @objc func toggleSidebar(_ sender: Any?) {
        bodyController.toggleSidebar(sender)
        selectedTab?.synchronizeSize(to: paneContainer)
    }

    // MARK: - Panes

    @objc func splitVertically(_ sender: Any?) {
        splitSelected(vertical: true)
    }

    @objc func splitHorizontally(_ sender: Any?) {
        splitSelected(vertical: false)
    }

    private func splitSelected(vertical: Bool) {
        guard let tab = selectedTab else { return }
        do {
            try tab.split(vertical: vertical)
            keepFindBarOnTop()
        } catch {
            reportPaneFailure(error)
        }
    }

    @objc func closeFocusedPane(_ sender: Any?) {
        selectedTab?.closeFocusedPane()
        keepFindBarOnTop()
    }

    @objc func focusNextPane(_ sender: Any?) {
        selectedTab?.cycleFocus(by: 1)
    }

    @objc func focusPreviousPane(_ sender: Any?) {
        selectedTab?.cycleFocus(by: -1)
    }

    /// The find bar must stay above the pane tree after any change to it.
    private func keepFindBarOnTop() {
        findBar.removeFromSuperview()
        paneContainer.addSubview(findBar, positioned: .above, relativeTo: nil)
    }

    // MARK: - Find

    @objc func showFindBar(_ sender: Any?) {
        findBar.focus()
    }

    @objc func findNext(_ sender: Any?) {
        findBar.repeatSearch(forward: true)
    }

    @objc func findPrevious(_ sender: Any?) {
        findBar.repeatSearch(forward: false)
    }

    // MARK: - Window

    /// Snap the window to whole cells, but only when a single pane fills the
    /// selected tab.
    ///
    /// With splits there is no single grid to snap to — dividers and rounding
    /// mean the panes cannot all land on cell boundaries at once. The sidebar's
    /// width is held out of the calculation for the same reason.
    func windowWillResize(_ sender: NSWindow, to frameSize: NSSize) -> NSSize {
        guard let tab = selectedTab, tab.panes.count == 1 else { return frameSize }

        let currentFrame = sender.frame
        let currentContent = sender.contentRect(forFrameRect: currentFrame).size
        let chrome = NSSize(
            width: currentFrame.width - currentContent.width,
            height: currentFrame.height - currentContent.height
        )
        let reserved = currentContent.width - paneContainer.bounds.width

        let proposedGrid = NSSize(
            width: frameSize.width - chrome.width - reserved,
            height: frameSize.height - chrome.height
        )
        let snapped = font.geometry.pixelSize(for: font.geometry.gridSize(fitting: proposedGrid))

        return NSSize(
            width: snapped.width + chrome.width + reserved,
            height: snapped.height + chrome.height
        )
    }

    func windowDidResize(_ notification: Notification) {
        // Every tab, not just the visible one: a background tab's content view
        // is out of the hierarchy so its bounds do not follow the window, and
        // skipping it would reflow a running full-screen program on every
        // switch instead of on the resize that actually caused it.
        for tab in tabs.values {
            tab.synchronizeSize(to: paneContainer)
        }
    }

    func windowWillClose(_ notification: Notification) {
        stop()
    }

    private func reportPaneFailure(_ error: Error) {
        let alert = NSAlert()
        alert.messageText = "newt could not start a terminal session"
        alert.informativeText = String(describing: error)
        alert.alertStyle = .warning
        alert.runModal()
    }
}
