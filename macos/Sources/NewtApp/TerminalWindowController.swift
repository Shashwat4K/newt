import AppKit
import Foundation
import NewtKit

/// Owns one window, its view, and the session behind it.
///
/// The session runs on its own thread inside the core; this class only samples
/// it. Sampling on the display link means the UI redraws at the screen's pace
/// no matter how fast the child floods the PTY — a `cat` of a large file
/// cannot outrun the renderer.
@MainActor
final class TerminalWindowController: NSObject, NSWindowDelegate {
    let window: NSWindow
    let view: TerminalView
    let session: TerminalSession
    private var displayLink: CADisplayLink?
    private var lastTitle: String?
    private var lastReportedSize: TerminalSize
    private let findBar = FindBar()

    init(cols: UInt16, rows: UInt16, fontSize: CGFloat) throws {
        let font = TerminalFont(size: fontSize)
        let initialSize = TerminalSize(cols: cols, rows: rows)
        session = try TerminalSession(size: initialSize)
        view = TerminalView(font: font, cols: Int(cols), rows: Int(rows))
        lastReportedSize = initialSize

        window = NSWindow(
            contentRect: view.frame,
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false
        )
        window.title = "newt"

        // The terminal fills the window; the find bar floats over its top-right
        // corner so opening it does not change the grid size.
        let container = NSView(frame: view.frame)
        view.autoresizingMask = [.width, .height]
        container.addSubview(view)

        findBar.isHidden = true
        findBar.autoresizingMask = [.minXMargin, .minYMargin]
        findBar.setFrameOrigin(
            NSPoint(x: view.frame.maxX - findBar.frame.width - 12, y: view.frame.maxY - findBar.frame.height - 12)
        )
        container.addSubview(findBar)

        window.contentView = container
        window.center()
        window.isReleasedWhenClosed = false

        super.init()
        window.delegate = self
        view.inputDelegate = self

        findBar.onFind = { [weak self] query, forward in
            guard let self else { return false }
            do {
                return try self.session.find(query, forward: forward)
            } catch {
                self.report(error)
                return false
            }
        }
        findBar.onClose = { [weak self] in
            guard let self else { return }
            self.window.makeFirstResponder(self.view)
        }

        // A window smaller than this cannot show anything useful, and letting
        // it shrink further just produces degenerate grids.
        window.contentMinSize = view.pixelSize(for: TerminalSize(cols: 20, rows: 5))

        // Report real cell metrics so the terminal can answer pixel-size
        // queries from full-screen programs.
        session.setCellSize(
            width: UInt16(font.cellWidth.rounded()),
            height: UInt16(font.cellHeight.rounded())
        )
    }

    /// Show the window and begin sampling the session.
    func start(runningCommand command: String?) {
        window.makeKeyAndOrderFront(nil)
        // Keystrokes reach the terminal only while the view holds focus.
        window.makeFirstResponder(view)

        let link = view.displayLink(target: self, selector: #selector(tick))
        link.add(to: .main, forMode: .common)
        displayLink = link

        if let command {
            // No input path yet (Phase 4), so a command can only be injected
            // here — enough to prove live output reaches the window.
            try? session.write("\(command)\n")
        }
    }

    func stop() {
        displayLink?.invalidate()
        displayLink = nil
    }

    @objc private func tick(_ link: CADisplayLink) {
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
            window.title = title
        }

        if session.hasExited {
            stop()
        }
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

    // MARK: - Resizing

    /// Snap the window to whole cells while it is being dragged.
    ///
    /// Without this the content view keeps a partial row or column that can
    /// never be drawn into, which reads as an uneven margin along one edge.
    func windowWillResize(_ sender: NSWindow, to frameSize: NSSize) -> NSSize {
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
        let snapped = view.pixelSize(for: view.gridSize(fitting: proposedContent))

        return NSSize(
            width: snapped.width + chrome.width,
            height: snapped.height + chrome.height
        )
    }

    func windowDidResize(_ notification: Notification) {
        resizeSessionToFitView()
    }

    /// Tell the core about a new grid size, but only when it actually changed.
    ///
    /// Resizing is a reflow of the whole scrollback and delivers SIGWINCH to
    /// the child; doing that on every pixel of a drag would be wasteful and
    /// would make programs redraw constantly.
    private func resizeSessionToFitView() {
        let grid = view.gridSize(fitting: view.bounds.size)
        guard grid != lastReportedSize else { return }
        lastReportedSize = grid

        do {
            try session.resize(to: grid)
        } catch {
            report(error)
        }
    }

    // MARK: - Input

    /// Input is forwarded verbatim; the core decides what bytes each event
    /// produces, because that depends on terminal modes it alone tracks.
    private func report(_ error: Error) {
        FileHandle.standardError.write(Data("newt: \(error)\n".utf8))
    }

    func windowWillClose(_ notification: Notification) {
        stop()
        NSApp.terminate(nil)
    }
}


extension TerminalWindowController: TerminalInputDelegate {
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
