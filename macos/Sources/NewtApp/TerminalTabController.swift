import AppKit
import Foundation
import NewtKit

/// One tab: a tree of split panes and the state the sidebar reads.
///
/// Extracted from `TerminalWindowController`, which conflated window, tab, and
/// pane tree. The split logic here is the Phase-7 code moved verbatim — it was
/// already correct — but it now belongs to a tab rather than to the window, so
/// several tabs can each own one.
///
/// An `NSObject` on purpose: `NSOutlineView` identifies its items with
/// `isEqual:`/`hash`, and a reference type gives it pointer identity. Never
/// hand an outline view a struct.
@MainActor
final class TerminalTabController: NSObject {
    let id: TabID
    let kind: TabKind

    /// Root of the pane tree. Retained whether or not it is in a window, which
    /// is what lets a background tab keep its layout and its sessions.
    let contentView = NSView()

    private let font: TerminalFont
    private let defaultSize: TerminalSize

    private(set) var panes: [TerminalPaneController] = []
    private(set) var focusedPane: TerminalPaneController?

    /// True while this tab's content is in the window.
    private(set) var isActive = false

    /// Called when a pane in this tab takes focus.
    var onFocusChange: ((TerminalTabController) -> Void)?
    /// Called when the last pane in this tab goes away.
    var onEmpty: ((TerminalTabController) -> Void)?
    /// Called when anything the sidebar draws may have changed.
    var onStatusChange: ((TerminalTabController) -> Void)?

    init(id: TabID, kind: TabKind, font: TerminalFont, size: TerminalSize) throws {
        self.id = id
        self.kind = kind
        self.font = font
        defaultSize = size

        super.init()

        let first = try TerminalPaneController(font: font, cols: size.cols, rows: size.rows)
        contentView.autoresizingMask = [.width, .height]
        contentView.frame = NSRect(origin: .zero, size: font.geometry.pixelSize(for: size))

        first.view.frame = contentView.bounds
        first.view.autoresizingMask = [.width, .height]
        contentView.addSubview(first.view)

        adopt(first)
        focusedPane = first
    }

    // MARK: - Title shown in the sidebar

    /// Title reported by whatever is running, if anything has.
    private(set) var reportedTitle: String?

    /// What the sidebar draws.
    ///
    /// Falls back rather than showing nothing: an agent's own title is best, a
    /// program's OSC title next, and the tab kind last. Phase 13 inserts the
    /// agent title at the front of this chain.
    var displayTitle: String {
        if let reportedTitle, !reportedTitle.isEmpty { return reportedTitle }
        switch kind {
        case .shell: return "shell"
        case .agent(let agent): return agent.displayName
        }
    }

    // MARK: - Activation

    /// Put this tab's panes on screen and start drawing them.
    func activate(in container: NSView) {
        contentView.frame = container.bounds
        container.addSubview(contentView)
        isActive = true
        for pane in panes {
            pane.resume()
        }
    }

    /// Take this tab off screen. Its sessions keep running.
    func deactivate() {
        for pane in panes {
            pane.suspend()
        }
        isActive = false
        contentView.removeFromSuperview()
    }

    func stop() {
        for pane in panes {
            pane.stop()
        }
    }

    /// Give keyboard focus to this tab's focused pane.
    func takeFocus(in window: NSWindow) {
        guard let pane = focusedPane ?? panes.first else { return }
        window.makeFirstResponder(pane.view)
    }

    /// Recompute every pane's grid from its bounds.
    ///
    /// Called for background tabs too: their content view is out of the
    /// hierarchy so its bounds do not follow the window, and skipping them
    /// would make every tab switch reflow a running full-screen program.
    func synchronizeSize(to container: NSView) {
        if !isActive {
            contentView.frame = container.bounds
            contentView.layoutSubtreeIfNeeded()
        }
        for pane in panes {
            pane.synchronizeSize()
        }
    }

    /// Poll every pane for title changes and exit. Runs for background tabs.
    func pollStatus() {
        for pane in panes {
            pane.pollStatus()
        }
    }

    // MARK: - Panes

    private func adopt(_ pane: TerminalPaneController) {
        panes.append(pane)
        pane.onFocus = { [weak self] pane in
            guard let self else { return }
            focusedPane = pane
            onFocusChange?(self)
        }
        pane.onExit = { [weak self] pane in self?.close(pane) }
        pane.onTitleChange = { [weak self] pane, title in
            // Only the focused pane names the tab; otherwise a background pane
            // would fight the one being looked at. Phase-7 decision 6, still.
            guard let self, focusedPane === pane else { return }
            reportedTitle = title
            onStatusChange?(self)
        }
    }

    /// Replace the focused pane with a split holding it and a new pane.
    ///
    /// - Parameter vertical: true puts the panes side by side.
    func split(vertical: Bool) throws {
        guard let focused = focusedPane, let superview = focused.view.superview else { return }

        let newPane = try TerminalPaneController(
            font: font,
            cols: defaultSize.cols,
            rows: defaultSize.rows
        )

        let splitView = NSSplitView(frame: focused.view.frame)
        splitView.isVertical = vertical
        splitView.dividerStyle = .thin
        splitView.autoresizingMask = focused.view.autoresizingMask

        // Put the split view exactly where the focused pane was, whether that
        // was directly in the container or inside another split.
        if let parent = superview as? NSSplitView {
            let index = parent.arrangedSubviews.firstIndex(of: focused.view) ?? 0
            focused.view.removeFromSuperview()
            parent.insertArrangedSubview(splitView, at: index)
        } else {
            focused.view.removeFromSuperview()
            superview.addSubview(splitView)
        }

        focused.view.autoresizingMask = [.width, .height]
        newPane.view.autoresizingMask = [.width, .height]
        newPane.view.frame = focused.view.bounds
        splitView.addArrangedSubview(focused.view)
        splitView.addArrangedSubview(newPane.view)

        // Place the divider explicitly rather than leaving it to
        // `adjustSubviews()`. That distributes proportionally to the frames the
        // subviews happen to have, which before the split view has been laid
        // out can be anything — including zero, which collapses the new pane to
        // an unusable 1x1 grid. An even split is also what ⌘D should mean.
        splitView.adjustSubviews()
        splitView.setPosition(
            (vertical ? splitView.bounds.width : splitView.bounds.height) / 2,
            ofDividerAt: 0
        )

        adopt(newPane)
        if isActive {
            newPane.start()
        }
        focusedPane = newPane
        newPane.view.window?.makeFirstResponder(newPane.view)
        onStatusChange?(self)
    }

    func closeFocusedPane() {
        guard let focused = focusedPane else { return }
        close(focused)
    }

    /// Remove a pane, collapsing any split left with a single child.
    func close(_ pane: TerminalPaneController) {
        guard let index = panes.firstIndex(where: { $0 === pane }) else { return }

        pane.stop()
        panes.remove(at: index)

        let superview = pane.view.superview
        pane.view.removeFromSuperview()

        // A split with one child left is no longer a split; collapsing it keeps
        // the tree from filling with single-child dividers.
        if let split = superview as? NSSplitView, split.arrangedSubviews.count == 1 {
            let survivor = split.arrangedSubviews[0]
            let frame = split.frame
            let mask = split.autoresizingMask
            let parent = split.superview

            survivor.removeFromSuperview()
            split.removeFromSuperview()

            survivor.frame = frame
            survivor.autoresizingMask = mask
            if let parentSplit = parent as? NSSplitView {
                parentSplit.addArrangedSubview(survivor)
                parentSplit.adjustSubviews()
            } else {
                parent?.addSubview(survivor)
            }
        }

        if panes.isEmpty {
            focusedPane = nil
            onEmpty?(self)
            return
        }

        focusedPane = panes.last
        if let next = focusedPane {
            next.view.window?.makeFirstResponder(next.view)
        }
        onStatusChange?(self)
    }

    func cycleFocus(by offset: Int) {
        guard !panes.isEmpty else { return }
        let current = panes.firstIndex(where: { $0 === focusedPane }) ?? 0
        let next = (current + offset + panes.count) % panes.count
        focusedPane = panes[next]
        panes[next].view.window?.makeFirstResponder(panes[next].view)
    }
}
