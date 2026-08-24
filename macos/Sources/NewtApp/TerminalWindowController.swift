import AppKit
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
    private let view: TerminalView
    private let session: TerminalSession
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

    func windowWillClose(_ notification: Notification) {
        stop()
        NSApp.terminate(nil)
    }
}
