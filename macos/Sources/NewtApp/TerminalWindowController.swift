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

    init(cols: UInt16, rows: UInt16, fontSize: CGFloat) throws {
        let font = TerminalFont(size: fontSize)
        session = try TerminalSession(size: TerminalSize(cols: cols, rows: rows))
        view = TerminalView(font: font, cols: Int(cols), rows: Int(rows))

        window = NSWindow(
            contentRect: view.frame,
            // Not resizable yet: reflow is Phase 5, and a window that resizes
            // without reflowing would just be broken.
            styleMask: [.titled, .closable, .miniaturizable],
            backing: .buffered,
            defer: false
        )
        window.title = "newt"
        window.contentView = view
        window.center()
        window.isReleasedWhenClosed = false

        super.init()
        window.delegate = self
        view.inputDelegate = self

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
}
