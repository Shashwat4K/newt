import AppKit
import Foundation
import NewtKit

/// One window: a tree of split panes, plus the find bar.
///
/// Panes are composed with `NSSplitView` rather than a hand-rolled layout tree —
/// the split view already handles dividers, dragging, and proportional resizing,
/// and its subview hierarchy *is* the tree.
@MainActor
final class TerminalWindowController: NSWindowController, NSWindowDelegate {
    /// Kept as a non-optional alongside `NSWindowController.window`, which is
    /// optional and would need unwrapping at every use.
    let hostWindow: NSWindow

    private let font: TerminalFont
    /// Holds the pane tree. The find bar floats above it.
    private let paneContainer = NSView()
    private let findBar = FindBar()

    private(set) var panes: [TerminalPaneController] = []
    private(set) var focusedPane: TerminalPaneController?

    /// Grid size used for panes created after the first.
    private let defaultSize: TerminalSize

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("newt does not use storyboards")
    }

    init(font: TerminalFont, cols: UInt16, rows: UInt16) throws {
        self.font = font
        defaultSize = TerminalSize(cols: cols, rows: rows)

        let first = try TerminalPaneController(font: font, cols: cols, rows: rows)
        let contentRect = NSRect(origin: .zero, size: font.geometry.pixelSize(for: defaultSize))

        hostWindow = NSWindow(
            contentRect: contentRect,
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false
        )
        hostWindow.title = "newt"
        hostWindow.contentMinSize = font.geometry.pixelSize(for: TerminalSize(cols: 20, rows: 5))
        // Opting into native window tabbing gives ⌘T, the tab bar, and tab
        // switching for free, rather than reimplementing all of it.
        hostWindow.tabbingMode = .preferred
        hostWindow.tabbingIdentifier = "newt.terminal"

        let container = NSView(frame: contentRect)
        paneContainer.frame = contentRect
        paneContainer.autoresizingMask = [.width, .height]
        container.addSubview(paneContainer)

        first.view.frame = paneContainer.bounds
        first.view.autoresizingMask = [.width, .height]
        paneContainer.addSubview(first.view)

        findBar.isHidden = true
        findBar.autoresizingMask = [.minXMargin, .minYMargin]
        findBar.setFrameOrigin(
            NSPoint(
                x: contentRect.maxX - findBar.frame.width - 12,
                y: contentRect.maxY - findBar.frame.height - 12
            )
        )
        container.addSubview(findBar)

        hostWindow.contentView = container

        // NSWindowController puts this object in the responder chain, which is
        // how menu commands like Split Right reach it.
        super.init(window: hostWindow)

        hostWindow.delegate = self
        adopt(first)
        focusedPane = first

        findBar.onFind = { [weak self] query, forward in
            guard let self, let pane = self.focusedPane else { return false }
            do {
                return try pane.session.find(query, forward: forward)
            } catch {
                pane.report(error)
                return false
            }
        }
        findBar.onClose = { [weak self] in
            guard let self, let pane = self.focusedPane else { return }
            self.hostWindow.makeFirstResponder(pane.view)
        }
    }

    /// Show the window and start every pane.
    func start(runningCommand command: String? = nil) {
        hostWindow.makeKeyAndOrderFront(nil)
        for pane in panes {
            pane.start()
        }
        if let pane = focusedPane {
            hostWindow.makeFirstResponder(pane.view)
            if let command {
                try? pane.session.write("\(command)\n")
            }
        }
    }

    func stop() {
        for pane in panes {
            pane.stop()
        }
    }

    // MARK: - Panes

    private func adopt(_ pane: TerminalPaneController) {
        panes.append(pane)
        pane.onFocus = { [weak self] pane in self?.focusedPane = pane }
        pane.onExit = { [weak self] pane in self?.close(pane) }
        pane.onTitleChange = { [weak self] pane, title in
            // Only the focused pane names the window; otherwise a background
            // pane's title would fight the one you are looking at.
            guard let self, self.focusedPane === pane else { return }
            self.hostWindow.title = title
        }
    }

    @objc func splitVertically(_ sender: Any?) {
        split(vertical: true)
    }

    @objc func splitHorizontally(_ sender: Any?) {
        split(vertical: false)
    }

    /// Replace the focused pane with a split holding it and a new pane.
    ///
    /// - Parameter vertical: true puts the panes side by side.
    private func split(vertical: Bool) {
        guard let focused = focusedPane, let superview = focused.view.superview else { return }

        let newPane: TerminalPaneController
        do {
            newPane = try TerminalPaneController(
                font: font,
                cols: defaultSize.cols,
                rows: defaultSize.rows
            )
        } catch {
            reportPaneFailure(error)
            return
        }

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
        splitView.addArrangedSubview(focused.view)
        splitView.addArrangedSubview(newPane.view)
        splitView.adjustSubviews()

        adopt(newPane)
        newPane.start()
        keepFindBarOnTop()

        hostWindow.makeFirstResponder(newPane.view)
    }

    @objc func closeFocusedPane(_ sender: Any?) {
        guard let focused = focusedPane else { return }
        close(focused)
    }

    /// Remove a pane, collapsing any split left with a single child.
    private func close(_ pane: TerminalPaneController) {
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

        keepFindBarOnTop()

        if panes.isEmpty {
            hostWindow.close()
            return
        }

        focusedPane = panes.last
        if let next = focusedPane {
            hostWindow.makeFirstResponder(next.view)
        }
    }

    @objc func focusNextPane(_ sender: Any?) {
        cycleFocus(by: 1)
    }

    @objc func focusPreviousPane(_ sender: Any?) {
        cycleFocus(by: -1)
    }

    private func cycleFocus(by offset: Int) {
        guard !panes.isEmpty else { return }
        let current = panes.firstIndex(where: { $0 === focusedPane }) ?? 0
        let next = (current + offset + panes.count) % panes.count
        hostWindow.makeFirstResponder(panes[next].view)
    }

    /// The find bar must stay above the pane tree after any change to it.
    private func keepFindBarOnTop() {
        findBar.removeFromSuperview()
        hostWindow.contentView?.addSubview(findBar, positioned: .above, relativeTo: nil)
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

    /// Snap the window to whole cells, but only when a single pane fills it.
    ///
    /// With splits there is no single grid to snap to — dividers and rounding
    /// mean the panes cannot all land on cell boundaries at once.
    func windowWillResize(_ sender: NSWindow, to frameSize: NSSize) -> NSSize {
        guard panes.count == 1 else { return frameSize }

        let currentFrame = sender.frame
        let currentContent = sender.contentRect(forFrameRect: currentFrame).size
        let chrome = NSSize(
            width: currentFrame.width - currentContent.width,
            height: currentFrame.height - currentContent.height
        )

        let proposedContent = NSSize(
            width: frameSize.width - chrome.width,
            height: frameSize.height - chrome.height
        )
        let snapped = font.geometry.pixelSize(for: font.geometry.gridSize(fitting: proposedContent))

        return NSSize(width: snapped.width + chrome.width, height: snapped.height + chrome.height)
    }

    func windowDidResize(_ notification: Notification) {
        for pane in panes {
            pane.synchronizeSize()
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
