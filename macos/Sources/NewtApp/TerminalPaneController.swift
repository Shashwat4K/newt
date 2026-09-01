import AppKit
import Foundation
import NewtKit

/// Owns one session and the view showing it.
///
/// A *pane* is the unit that pairs a core session with its UI state — the seam
/// the plan calls for. Tabs and splits compose panes; nothing above this layer
/// touches a session directly.
@MainActor
final class TerminalPaneController: NSObject {
    let view: TerminalView
    let session: TerminalSession
    let font: TerminalFont

    /// Called when this pane's session ends, so the window can close it.
    var onExit: ((TerminalPaneController) -> Void)?
    /// Called when the child sets a new window title.
    var onTitleChange: ((TerminalPaneController, String) -> Void)?
    /// Called when this pane takes keyboard focus.
    var onFocus: ((TerminalPaneController) -> Void)?

    private var displayLink: CADisplayLink?
    private var lastTitle: String?
    private var lastReportedSize: TerminalSize

    init(font: TerminalFont, cols: UInt16, rows: UInt16) throws {
        self.font = font
        let initialSize = TerminalSize(cols: cols, rows: rows)
        session = try TerminalSession(size: initialSize)
        view = TerminalView(font: font, cols: Int(cols), rows: Int(rows))
        lastReportedSize = initialSize

        super.init()
        view.inputDelegate = self

        session.setCellSize(
            width: UInt16(font.cellWidth.rounded()),
            height: UInt16(font.cellHeight.rounded())
        )
    }

    /// Begin sampling the session. Safe to call once the view has a window.
    func start() {
        guard displayLink == nil else { return }
        let link = view.displayLink(target: self, selector: #selector(tick))
        link.add(to: .main, forMode: .common)
        displayLink = link
    }

    /// Stop drawing, without touching the session.
    ///
    /// This is what a background tab costs: nothing. The PTY reader thread, the
    /// parser, and the grid all live in the core and are entirely independent
    /// of the display link, so a suspended pane keeps consuming output and
    /// simply stops copying it to the screen. Nothing is buffered and nothing
    /// is lost — `GridBuffer` copies the whole grid on the next `apply`.
    func suspend() {
        displayLink?.invalidate()
        displayLink = nil
    }

    /// Resume drawing after `suspend()`, repainting from scratch.
    func resume() {
        view.needsFullRedraw = true
        start()
    }

    func stop() {
        suspend()
    }

    /// Recompute the grid from the view's bounds and tell the core.
    ///
    /// A resize reflows the whole scrollback and delivers SIGWINCH, so it only
    /// happens when the grid size actually changed.
    func synchronizeSize() {
        let grid = font.geometry.gridSize(fitting: view.bounds.size)
        guard grid != lastReportedSize else { return }
        lastReportedSize = grid

        // A reflow rewrites the whole grid, and the view's bounds just changed
        // under it, so the damage list no longer describes what is on screen.
        // Without this a pane that has just been split keeps whatever it drew
        // at its old size, which reads as a nearly blank pane.
        view.needsFullRedraw = true

        do {
            try session.resize(to: grid)
        } catch {
            report(error)
        }
    }

    /// Copy the current grid into the view immediately, outside the tick.
    ///
    /// The window never needs this — its display link runs at screen rate, so
    /// the view is at most a frame behind. Offscreen capture does: it draws
    /// straight from `GridBuffer`, and without a tick between the last resize
    /// and `cacheDisplay` it photographs a stale buffer while the core already
    /// holds the right grid. That made `--render-to` intermittently produce
    /// blank panes, which is worse than useless in a verification tool.
    func refreshNow() {
        try? session.withSnapshot { view.apply($0) }
    }

    /// Draw the current frame. Foreground panes only — this is the render tick.
    @objc private func tick(_ link: CADisplayLink) {
        // The view may have been resized by the split view since the last tick.
        synchronizeSize()

        do {
            try session.withSnapshot { snapshot in
                view.apply(snapshot)
            }
        } catch {
            suspend()
        }
    }

    /// Report title changes and process exit. Called by the app-wide status
    /// ticker for *every* pane, foreground or not.
    ///
    /// This deliberately does not live in `tick`: a background tab's display
    /// link is suspended, so a shell that exits there would otherwise never be
    /// noticed and its tab would sit in the sidebar forever.
    func pollStatus() {
        if let title = session.title, title != lastTitle {
            lastTitle = title
            onTitleChange?(self, title)
        }

        if session.hasExited {
            suspend()
            onExit?(self)
        }
    }

    func report(_ error: Error) {
        FileHandle.standardError.write(Data("newt: \(error)\n".utf8))
    }
}

extension TerminalPaneController: TerminalInputDelegate {
    func terminalViewDidBecomeFocused(_ view: TerminalView) {
        onFocus?(self)
    }

    func terminalView(_ view: TerminalView, send key: TerminalKey, modifiers: KeyModifiers) {
        do { try session.send(key: key, modifiers: modifiers) } catch { report(error) }
    }

    func terminalView(_ view: TerminalView, sendText text: String) {
        do { try session.send(text: text) } catch { report(error) }
    }

    @discardableResult
    func terminalView(
        _ view: TerminalView,
        sendMouse kind: MouseEventKind,
        button: MouseButton,
        col: UInt16,
        row: UInt16,
        modifiers: KeyModifiers
    ) -> Bool {
        do {
            return try session.send(
                mouse: kind,
                button: button,
                col: col,
                row: row,
                modifiers: modifiers
            )
        } catch {
            report(error)
            return false
        }
    }

    func terminalView(_ view: TerminalView, scrollByLines lines: Int32) {
        session.scroll(by: lines)
    }

    func terminalView(_ view: TerminalView, paste text: String) {
        do { try session.paste(text) } catch { report(error) }
    }

    func terminalView(
        _ view: TerminalView,
        startSelection col: UInt16,
        row: UInt16,
        sideRight: Bool,
        mode: SelectionMode
    ) {
        do {
            try session.startSelection(col: col, row: row, sideRight: sideRight, mode: mode)
        } catch {
            report(error)
        }
    }

    func terminalView(
        _ view: TerminalView,
        updateSelection col: UInt16,
        row: UInt16,
        sideRight: Bool
    ) {
        do {
            try session.updateSelection(col: col, row: row, sideRight: sideRight)
        } catch {
            report(error)
        }
    }

    func terminalViewSelectedText(_ view: TerminalView) -> String? {
        session.selectedText
    }

    func terminalViewScrollToBottom(_ view: TerminalView) {
        session.scrollToBottom()
    }
}
