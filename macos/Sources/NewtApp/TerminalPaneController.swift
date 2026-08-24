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

    func stop() {
        displayLink?.invalidate()
        displayLink = nil
    }

    /// Recompute the grid from the view's bounds and tell the core.
    ///
    /// A resize reflows the whole scrollback and delivers SIGWINCH, so it only
    /// happens when the grid size actually changed.
    func synchronizeSize() {
        let grid = font.geometry.gridSize(fitting: view.bounds.size)
        guard grid != lastReportedSize else { return }
        lastReportedSize = grid

        do {
            try session.resize(to: grid)
        } catch {
            report(error)
        }
    }

    @objc private func tick(_ link: CADisplayLink) {
        // The view may have been resized by the split view since the last tick.
        synchronizeSize()

        do {
            try session.withSnapshot { snapshot in
                view.apply(snapshot)
            }
        } catch {
            stop()
            return
        }

        if let title = session.title, title != lastTitle {
            lastTitle = title
            onTitleChange?(self, title)
        }

        if session.hasExited {
            stop()
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
