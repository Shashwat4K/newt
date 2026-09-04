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
final class TerminalTabController: NSObject, NSSplitViewDelegate {
    let id: TabID
    let kind: TabKind

    /// Root of the pane tree. Retained whether or not it is in a window, which
    /// is what lets a background tab keep its layout and its sessions.
    let contentView = NSView()

    private let font: TerminalFont
    private let defaultSize: TerminalSize
    /// What every pane in this tab runs.
    ///
    /// Held rather than rebuilt so a pane added by a split is the same kind of
    /// thing as the one it was split from.
    private let spec: SessionSpec

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

    init(
        id: TabID,
        kind: TabKind,
        font: TerminalFont,
        size: TerminalSize,
        spec: SessionSpec
    ) throws {
        self.id = id
        self.kind = kind
        self.font = font
        self.spec = spec
        defaultSize = size

        super.init()

        let first = try TerminalPaneController(
            font: font,
            cols: size.cols,
            rows: size.rows,
            spec: spec
        )
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
    /// Falls back rather than showing nothing: the agent's own name for what
    /// it is doing is best, the program's OSC title next, and the tab kind
    /// last. The agent title only appears once the agent reports one, which is
    /// why the chain exists rather than a single source.
    var displayTitle: String {
        if let agentTitle = focusedPane?.session.metadata.agentTitle, !agentTitle.isEmpty {
            return agentTitle
        }
        if let reportedTitle, !reportedTitle.isEmpty { return reportedTitle }
        switch kind {
        case .shell: return "shell"
        case .agent(let agent): return agent.displayName
        }
    }

    /// The agent session a child tab would fork from, once one is known.
    var agentSessionID: String? {
        focusedPane?.session.metadata.agentSessionID
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

    /// What the sidebar last drew for this tab, so a change can be noticed.
    private var lastObservedMetadata = SessionMetadata()

    /// Poll every pane for title changes, exit, and agent state.
    ///
    /// Runs for background tabs too, and the metadata half is why this exists
    /// rather than leaving it to the title. Agent state, tokens and cost do not
    /// change the terminal title, so before this the row was only repainted
    /// when the title happened to change or something rebuilt the whole
    /// sidebar — which made an indicator look like it belonged to whichever
    /// tab was touched last, rather than to its own session.
    func pollStatus() {
        for pane in panes {
            pane.pollStatus()
        }

        let current = focusedPane?.session.metadata ?? SessionMetadata()
        if current != lastObservedMetadata {
            lastObservedMetadata = current
            onStatusChange?(self)
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

        // A split of an agent tab starts another agent, not a shell: the pane
        // is a second view onto the same kind of work.
        let newPane = try TerminalPaneController(
            font: font,
            cols: defaultSize.cols,
            rows: defaultSize.rows,
            spec: spec
        )

        let splitView = NSSplitView(frame: focused.view.frame)
        splitView.isVertical = vertical
        splitView.dividerStyle = .thin
        splitView.delegate = self
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

    // MARK: - Pane layout

    /// Smallest a pane may be squeezed to, in points.
    ///
    /// A pane below this cannot show anything useful, and one at zero is worse
    /// than useless — it still holds a live session that can never be seen.
    private static let minimumPaneExtent: CGFloat = 40

    /// Distribute a split view's space explicitly.
    ///
    /// `adjustSubviews()` is not used, because its proportional distribution
    /// needs meaningful previous sizes and does not always have them: during a
    /// layout pass the container can be momentarily zero-width, and dividing by
    /// that gave one pane everything and the other nothing. The panes survived
    /// the split and then collapsed on the next layout, which looked like the
    /// split failing.
    ///
    /// Doing the arithmetic here makes it deterministic and lets a pane keep a
    /// floor, so neither AppKit nor a drag can squeeze one out of existence.
    func splitView(_ splitView: NSSplitView, resizeSubviewsWithOldSize oldSize: NSSize) {
        let views = splitView.arrangedSubviews
        guard views.count == 2 else {
            splitView.adjustSubviews()
            return
        }

        let vertical = splitView.isVertical
        let bounds = splitView.bounds
        let total = vertical ? bounds.width : bounds.height
        let available = max(0, total - splitView.dividerThickness)

        let first = views[0].frame
        let second = views[1].frame
        let previous =
            vertical
            ? first.width + second.width
            : first.height + second.height

        // No usable proportion to preserve: split it down the middle rather
        // than inventing one.
        let share =
            previous > 0
            ? available * ((vertical ? first.width : first.height) / previous)
            : available / 2

        let floor = min(Self.minimumPaneExtent, available / 2)
        let firstExtent = min(max(share.rounded(), floor), available - floor)
        let secondExtent = available - firstExtent

        if vertical {
            views[0].frame = NSRect(x: 0, y: 0, width: firstExtent, height: bounds.height)
            views[1].frame = NSRect(
                x: firstExtent + splitView.dividerThickness,
                y: 0,
                width: secondExtent,
                height: bounds.height
            )
        } else {
            // Subview 0 is the top one, and the coordinate system is not
            // flipped, so it is the one with the higher origin.
            views[0].frame = NSRect(
                x: 0,
                y: secondExtent + splitView.dividerThickness,
                width: bounds.width,
                height: firstExtent
            )
            views[1].frame = NSRect(x: 0, y: 0, width: bounds.width, height: secondExtent)
        }
    }

    func cycleFocus(by offset: Int) {
        guard !panes.isEmpty else { return }
        let current = panes.firstIndex(where: { $0 === focusedPane }) ?? 0
        let next = (current + offset + panes.count) % panes.count
        focusedPane = panes[next]
        panes[next].view.window?.makeFirstResponder(panes[next].view)
    }
}
